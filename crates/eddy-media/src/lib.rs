//! Turns the link of an item into media that can be shown or played
//! directly: images, video files and streams, embeddable players. Only
//! what sites hand to anonymous visitors is read; pages that need an
//! account report it instead (see docs/adr/0007).

mod feed;
mod link;
mod reddit;
mod redgifs;

use serde::{Deserialize, Serialize};

pub use feed::{feed_media, feed_thumbnail};
pub use link::{Link, is_media_url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    /// A video file or an HLS stream.
    Video,
    /// A third-party player page (YouTube, Vimeo) shown in a web view.
    Embed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub kind: MediaType,
    pub src: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub has_audio: bool,
    /// `youtube`, `vimeo` for embeds.
    #[serde(default)]
    pub provider: Option<String>,
}

impl MediaItem {
    pub fn image(src: impl Into<String>) -> MediaItem {
        MediaItem {
            kind: MediaType::Image,
            src: src.into(),
            poster: None,
            width: None,
            height: None,
            duration: None,
            has_audio: false,
            provider: None,
        }
    }

    pub fn video(src: impl Into<String>) -> MediaItem {
        MediaItem {
            kind: MediaType::Video,
            has_audio: true,
            ..MediaItem::image(src)
        }
    }

    /// The picture that stands for this item in a grid.
    pub fn preview(&self) -> Option<&str> {
        match self.kind {
            MediaType::Image => Some(&self.src),
            _ => self.poster.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    Ready,
    /// The item has nothing to show or play.
    NoMedia,
    /// The page hides its media from anonymous visitors.
    NeedsSignIn,
    /// The site could not be reached or answered unexpectedly; worth
    /// trying again later.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaSet {
    pub status: MediaStatus,
    pub items: Vec<MediaItem>,
    /// Grid picture: the feed's own thumbnail when there is one (small and
    /// already at hand), else the first item's preview.
    pub thumbnail: Option<String>,
}

/// Fetches pages for resolvers that read them (Reddit posts). The host
/// uses its fetch pipeline, so a signed-in WebView session applies.
#[async_trait::async_trait]
pub trait PageSource: Send + Sync {
    async fn page(&self, url: &str) -> Option<String>;
}

enum Miss {
    SignIn,
    Failed,
}

pub struct Resolver {
    redgifs: redgifs::RedGifs,
}

impl Default for Resolver {
    fn default() -> Self {
        Resolver::new()
    }
}

impl Resolver {
    pub fn new() -> Resolver {
        Resolver {
            redgifs: redgifs::RedGifs::new("https://api.redgifs.com"),
        }
    }

    /// Points RedGIFs lookups at another host (tests).
    pub fn with_redgifs_api(mut self, base: impl Into<String>) -> Resolver {
        self.redgifs = redgifs::RedGifs::new(base);
        self
    }

    /// Resolves `url`, the item's link. `feed_html` is the content the
    /// feed delivered, used as is when it already carries the media and as
    /// the fallback when the site fails.
    pub async fn resolve(
        &self,
        url: &str,
        feed_html: Option<&str>,
        pages: &dyn PageSource,
    ) -> MediaSet {
        let feed = feed_html.map(feed_media).unwrap_or_default();
        let found = self.resolve_link(&Link::parse(url), &feed, pages, 0).await;
        finish(found, feed)
    }

    async fn resolve_link(
        &self,
        link: &Link,
        feed: &[MediaItem],
        pages: &dyn PageSource,
        depth: u8,
    ) -> Result<Vec<MediaItem>, Miss> {
        match link {
            Link::File(item) => Ok(vec![item.clone()]),
            Link::RedGifs(id) => self.redgifs.resolve(id).await,
            Link::YouTube(id) => Ok(vec![link::youtube(id)]),
            Link::Vimeo(id) => Ok(vec![link::vimeo(id)]),
            Link::RedditVideo(id) => Ok(vec![reddit::hls(id, None)]),
            Link::RedditPost { id, gallery } => {
                // Feeds often carry the full picture of single-image posts.
                if !gallery && feed.iter().any(|m| reddit::is_full_size(&m.src)) {
                    return Ok(feed.to_vec());
                }
                let html = pages
                    .page(&format!("https://www.reddit.com/comments/{id}/"))
                    .await
                    .ok_or(Miss::Failed)?;
                match reddit::parse_post(&html) {
                    reddit::Post::Media(items) => Ok(items),
                    reddit::Post::Link(href) if depth == 0 => {
                        let inner = Link::parse(&href);
                        if matches!(inner, Link::Other | Link::RedditPost { .. }) {
                            Ok(Vec::new())
                        } else {
                            Box::pin(self.resolve_link(&inner, feed, pages, 1)).await
                        }
                    }
                    reddit::Post::Link(_) | reddit::Post::Text => Ok(Vec::new()),
                    reddit::Post::SignIn => Err(Miss::SignIn),
                    reddit::Post::Blocked => Err(Miss::Failed),
                }
            }
            Link::Other => Ok(Vec::new()),
        }
    }
}

fn finish(found: Result<Vec<MediaItem>, Miss>, feed: Vec<MediaItem>) -> MediaSet {
    let feed_thumb = feed::thumbnail(&feed);
    let (status, items) = match found {
        Ok(items) if !items.is_empty() => (MediaStatus::Ready, items),
        Ok(_) if !feed.is_empty() => (MediaStatus::Ready, feed),
        Ok(_) => (MediaStatus::NoMedia, Vec::new()),
        Err(Miss::SignIn) => (MediaStatus::NeedsSignIn, feed),
        Err(Miss::Failed) => (MediaStatus::Failed, feed),
    };
    let thumbnail = feed_thumb.or_else(|| items.iter().find_map(|m| m.preview().map(String::from)));
    MediaSet {
        status,
        items,
        thumbnail,
    }
}

#[cfg(test)]
mod tests;
