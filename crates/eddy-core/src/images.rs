use std::collections::BTreeMap;
use std::sync::Arc;

use eddy_model::ids::sha256_hex;
use eddy_model::{BlockKind, Document, EPUB_SCHEME, PDF_SCHEME};

use crate::engine::Inner;
use crate::import::parse_pdf_image_url;
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
        let key = image_blob_key(url);
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

    /// Renders the figures of a PDF document into the image cache in the
    /// background, a page at a time, so opening it shows them at once
    /// instead of rendering each as the reader reaches it. Figures already
    /// cached are skipped; a PDF being warmed is not started again. Without
    /// a PDF reader or a runtime (servers, tests of the store) nothing runs.
    pub(crate) fn warm_pdf_images(self: &Arc<Self>, document: &Document) {
        let reader = self.pdf_reader.read().clone();
        let Some(reader) = reader else { return };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Per PDF file: the pages and object indices still to render.
        let mut wanted: BTreeMap<String, Vec<(u32, u32)>> = BTreeMap::new();
        for block in &document.blocks {
            let BlockKind::Image { src, .. } = &block.kind else {
                continue;
            };
            let Some((file, page, index)) = parse_pdf_image_url(src) else {
                continue;
            };
            if self.store.has_blob(&image_blob_key(src)).unwrap_or(true) {
                continue;
            }
            wanted.entry(file).or_default().push((page, index));
        }
        for (file, mut requests) in wanted {
            if !self.pdf_warming.lock().insert(file.clone()) {
                continue;
            }
            requests.sort_unstable();
            requests.dedup();
            let inner = self.clone();
            let reader = reader.clone();
            handle.spawn(async move {
                let path = match inner.local_blob_file(&file).await {
                    Ok(Some(path)) => Some(path),
                    _ => None,
                };
                if let Some(path) = path {
                    inner
                        .render_pdf_figures(&reader, &file, &path, requests)
                        .await;
                }
                inner.pdf_warming.lock().remove(&file);
            });
        }
    }

    /// Renders `requests` from one PDF page by page, storing each page's
    /// pictures as soon as it is done.
    async fn render_pdf_figures(
        &self,
        reader: &Arc<dyn crate::PdfReader>,
        file: &str,
        path: &std::path::Path,
        requests: Vec<(u32, u32)>,
    ) {
        let mut pages: Vec<Vec<(u32, u32)>> = Vec::new();
        for request in requests {
            match pages.last_mut() {
                Some(page) if page[0].0 == request.0 => page.push(request),
                _ => pages.push(vec![request]),
            }
        }
        for page in pages {
            let reader = reader.clone();
            let path = path.to_path_buf();
            let batch = page.clone();
            let Ok(pictures) =
                tokio::task::spawn_blocking(move || reader.images(&path, &batch)).await
            else {
                return;
            };
            for ((page, index), picture) in page.into_iter().zip(pictures) {
                let Some((bytes, mime)) = picture else {
                    continue;
                };
                if bytes.len() > MAX_IMAGE_BYTES {
                    continue;
                }
                let url = format!("{PDF_SCHEME}://{file}/{page}/{index}");
                let _ = self
                    .store
                    .put_blob_keyed(&image_blob_key(&url), &bytes, mime, false);
            }
            // Leave room for the reader's own requests between pages.
            tokio::task::yield_now().await;
        }
    }
}

/// Blob key of a cached image. What a PDF image address shows (which strips
/// and layers make one picture) is decided by the document model, so those
/// version with it.
pub(crate) fn image_blob_key(url: &str) -> String {
    let cache_name = if url.starts_with(&format!("{PDF_SCHEME}://")) {
        format!("{url}#{}", eddy_doc::SCHEMA_VERSION)
    } else {
        url.to_string()
    };
    format!("img{}", &sha256_hex(cache_name.as_bytes())[..40])
}
