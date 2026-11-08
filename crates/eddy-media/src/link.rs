use url::Url;

use crate::{MediaItem, MediaType};

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "avif", "heic"];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mov", "m4v", "m3u8"];

/// What an item's link points at, as far as the link alone tells.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// An image or video file.
    File(MediaItem),
    RedGifs(String),
    YouTube(String),
    Vimeo(String),
    /// A video hosted by Reddit (`v.redd.it/{id}`).
    RedditVideo(String),
    /// A Reddit post; what it holds is on its page.
    RedditPost {
        id: String,
        gallery: bool,
    },
    Other,
}

impl Link {
    pub fn parse(url: &str) -> Link {
        let Ok(parsed) = Url::parse(url.trim()) else {
            return Link::Other;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return Link::Other;
        }
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        let segments: Vec<&str> = parsed
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();
        let site = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));

        if site("redgifs.com") && host != "media.redgifs.com" {
            return match segments.as_slice() {
                ["watch" | "ifr" | "i", id, ..] => {
                    let id = id.split('.').next().unwrap_or(id).to_ascii_lowercase();
                    let id = id.trim_end_matches("-mobile").to_string();
                    if id.is_empty() {
                        Link::Other
                    } else {
                        Link::RedGifs(id)
                    }
                }
                _ => Link::Other,
            };
        }
        if site("youtube.com") || site("youtube-nocookie.com") {
            let id = match segments.as_slice() {
                ["watch"] => parsed
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned()),
                ["shorts" | "embed" | "live" | "v", id, ..] => Some(id.to_string()),
                _ => None,
            };
            return id
                .filter(|i| valid_youtube_id(i))
                .map_or(Link::Other, Link::YouTube);
        }
        if host == "youtu.be" {
            return segments
                .first()
                .filter(|i| valid_youtube_id(i))
                .map_or(Link::Other, |i| Link::YouTube(i.to_string()));
        }
        if site("vimeo.com") {
            let id = match segments.as_slice() {
                ["video", id, ..] => Some(*id),
                [id, ..] => Some(*id),
                _ => None,
            };
            return id
                .filter(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()))
                .map_or(Link::Other, |i| Link::Vimeo(i.to_string()));
        }
        if host == "v.redd.it" {
            return segments
                .first()
                .map_or(Link::Other, |i| Link::RedditVideo(i.to_string()));
        }
        if site("reddit.com") {
            match segments.as_slice() {
                ["gallery", id, ..] => {
                    return Link::RedditPost {
                        id: id.to_string(),
                        gallery: true,
                    };
                }
                ["r", _, "comments", id, ..] | ["comments", id, ..] => {
                    return Link::RedditPost {
                        id: id.to_string(),
                        gallery: false,
                    };
                }
                // Reddit's media viewer wraps the file it shows.
                ["media"] => {
                    if let Some((_, inner)) = parsed.query_pairs().find(|(k, _)| k == "url") {
                        return Link::parse(&inner);
                    }
                }
                _ => {}
            }
            return Link::Other;
        }
        let path = parsed.path().to_ascii_lowercase();
        let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        if ext == "gifv" {
            let mut mp4 = parsed.clone();
            let original = parsed.path();
            mp4.set_path(&format!(
                "{}mp4",
                &original[..original.len() - "gifv".len()]
            ));
            return Link::File(MediaItem {
                has_audio: false,
                ..MediaItem::video(mp4.as_str())
            });
        }
        if IMAGE_EXTENSIONS.contains(&ext) {
            return Link::File(MediaItem::image(parsed.as_str()));
        }
        if VIDEO_EXTENSIONS.contains(&ext) {
            return Link::File(MediaItem::video(parsed.as_str()));
        }
        Link::Other
    }
}

fn valid_youtube_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Links that are media themselves: saved ones belong to the Media part of
/// the collection, and fetching "full text" for them is pointless.
pub fn is_media_url(url: &str) -> bool {
    match Link::parse(url) {
        Link::RedditPost { gallery, .. } => gallery,
        Link::Other => false,
        _ => true,
    }
}

pub(crate) fn youtube(id: &str) -> MediaItem {
    MediaItem {
        kind: MediaType::Embed,
        poster: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        provider: Some("youtube".into()),
        ..MediaItem::video(format!(
            "https://www.youtube-nocookie.com/embed/{id}?autoplay=1&playsinline=1"
        ))
    }
}

pub(crate) fn vimeo(id: &str) -> MediaItem {
    MediaItem {
        kind: MediaType::Embed,
        provider: Some("vimeo".into()),
        ..MediaItem::video(format!("https://player.vimeo.com/video/{id}?autoplay=1"))
    }
}
