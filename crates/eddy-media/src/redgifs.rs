use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::Deserialize;

use crate::{MediaItem, Miss};

/// RedGIFs' public API with the temporary token its website hands every
/// anonymous visitor. The token is tied to the user agent.
pub(crate) struct RedGifs {
    http: reqwest::Client,
    base: String,
    token: parking_lot::Mutex<Option<(String, Instant)>>,
}

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const TOKEN_LIFETIME: Duration = Duration::from_secs(12 * 3600);

#[derive(Deserialize)]
struct TokenReply {
    token: String,
}

#[derive(Deserialize)]
struct GifReply {
    gif: Gif,
}

#[derive(Deserialize)]
struct GalleryReply {
    gifs: Vec<Gif>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Gif {
    #[serde(default, rename = "type")]
    kind: Option<u8>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    has_audio: bool,
    #[serde(default)]
    gallery: Option<String>,
    urls: Urls,
}

#[derive(Deserialize)]
struct Urls {
    #[serde(default)]
    hd: Option<String>,
    #[serde(default)]
    sd: Option<String>,
    #[serde(default)]
    poster: Option<String>,
}

impl Gif {
    fn into_item(self) -> Option<MediaItem> {
        let src = self.urls.hd.or(self.urls.sd)?;
        let image = self.kind == Some(2) || self.duration.is_none() && !src.ends_with(".mp4");
        let mut item = if image {
            MediaItem::image(src)
        } else {
            MediaItem {
                poster: self.urls.poster,
                duration: self.duration,
                has_audio: self.has_audio,
                ..MediaItem::video(src)
            }
        };
        item.width = self.width;
        item.height = self.height;
        Some(item)
    }
}

impl RedGifs {
    pub(crate) fn new(base: impl Into<String>) -> RedGifs {
        RedGifs {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
            base: base.into().trim_end_matches('/').to_string(),
            token: parking_lot::Mutex::new(None),
        }
    }

    pub(crate) async fn resolve(&self, id: &str) -> Result<Vec<MediaItem>, Miss> {
        let gif: GifReply = match self.get(&format!("/v2/gifs/{id}")).await? {
            Some(reply) => reply,
            None => return Ok(Vec::new()),
        };
        if let Some(gallery) = gif.gif.gallery.clone()
            && let Some(reply) = self
                .get::<GalleryReply>(&format!("/v2/gallery/{gallery}"))
                .await?
            && !reply.gifs.is_empty()
        {
            return Ok(reply.gifs.into_iter().filter_map(Gif::into_item).collect());
        }
        Ok(gif.gif.into_item().into_iter().collect())
    }

    /// `None` when the gif no longer exists.
    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<Option<T>, Miss> {
        for attempt in 0..2 {
            let token = self.token(attempt > 0).await?;
            let response = self
                .http
                .get(format!("{}{path}", self.base))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| Miss::Failed)?;
            match response.status() {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN if attempt == 0 => continue,
                StatusCode::NOT_FOUND | StatusCode::GONE => return Ok(None),
                s if s.is_success() => {
                    return response.json().await.map(Some).map_err(|_| Miss::Failed);
                }
                _ => return Err(Miss::Failed),
            }
        }
        Err(Miss::Failed)
    }

    async fn token(&self, renew: bool) -> Result<String, Miss> {
        if !renew
            && let Some((token, at)) = self.token.lock().clone()
            && at.elapsed() < TOKEN_LIFETIME
        {
            return Ok(token);
        }
        let reply: TokenReply = self
            .http
            .get(format!("{}/v2/auth/temporary", self.base))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| Miss::Failed)?
            .json()
            .await
            .map_err(|_| Miss::Failed)?;
        *self.token.lock() = Some((reply.token.clone(), Instant::now()));
        Ok(reply.token)
    }
}
