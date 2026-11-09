use eddy_model::{EPUB_SCHEME, PDF_SCHEME};
use eddy_model::ids::sha256_hex;

use crate::engine::Inner;
use crate::{EngineError, Result};

const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;
/// Animated GIFs run long: Reddit's reach 100 MB.
const MAX_GIF_BYTES: usize = 120 * 1024 * 1024;

impl Inner {
    /// Image bytes for `url`, cached as a blob so articles render offline.
    pub(crate) async fn image(&self, url: &str) -> Result<(Vec<u8>, String)> {
        let parsed =
            url::Url::parse(url).map_err(|_| EngineError::Invalid("图片地址无效。".into()))?;
        if !matches!(parsed.scheme(), "http" | "https" | EPUB_SCHEME | PDF_SCHEME) {
            return Err(EngineError::Invalid("图片地址无效。".into()));
        }
        let key = format!("img{}", &sha256_hex(url.as_bytes())[..40]);
        if let Some(bytes) = self.store.get_blob(&key)? {
            let mime = self
                .store
                .blob_mime(&key)?
                .unwrap_or_else(|| "image/*".into());
            return Ok((bytes, mime));
        }
        if matches!(parsed.scheme(), EPUB_SCHEME | PDF_SCHEME) {
            // Not pinned: it can be read from the book or PDF again.
            let found = if parsed.scheme() == EPUB_SCHEME {
                self.archive_file(url).await?
            } else {
                self.pdf_image(url).await?
            };
            let (bytes, mime) = found.ok_or(EngineError::NotFound)?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(EngineError::Invalid("图片太大。".into()));
            }
            self.store.put_blob_keyed(&key, &bytes, mime, false)?;
            return Ok((bytes, mime.to_string()));
        }
        let response = self
            .http
            .get(parsed)
            // Room for a large GIF on a slow connection.
            .timeout(std::time::Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        if !response.status().is_success() {
            return Err(EngineError::Network(format!(
                "HTTP {}",
                response.status().as_u16()
            )));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/*")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let limit = if mime.contains("gif") {
            MAX_GIF_BYTES
        } else {
            MAX_IMAGE_BYTES
        };
        if bytes.len() > limit {
            return Err(EngineError::Invalid("图片太大。".into()));
        }
        self.store.put_blob_keyed(&key, &bytes, &mime, false)?;
        Ok((bytes.to_vec(), mime))
    }
}
