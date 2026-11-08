use std::sync::LazyLock;

use regex::Regex;

use crate::MediaItem;

/// What a Reddit post page holds.
pub(crate) enum Post {
    Media(Vec<MediaItem>),
    /// A link post pointing elsewhere.
    Link(String),
    Text,
    SignIn,
    /// A bot check instead of the page.
    Blocked,
}

static POST_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<shreddit-post\b[^>]*>").unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"([\w-]+)="([^"]*)""#).unwrap());
static PAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<li\b[^>]*\bslot="page-\d+""#).unwrap());
static FULL_IMAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"src="(https://i\.redd\.it/[^"]+)""#).unwrap());
static ANY_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:data-lazy-src|src)="(https://[\w.]*redd\.it/[^"]+)""#).unwrap()
});
static SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"width="(\d+)"\s+height="(\d+)""#).unwrap());
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://v\.redd\.it/([a-z0-9]+)/").unwrap());
static PLAYER_POSTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<shreddit-player[\w-]*\b[^>]*?\bposter="([^"]+)""#).unwrap()
});

pub(crate) fn parse_post(html: &str) -> Post {
    let Some(tag) = POST_TAG.find(html) else {
        return if eddy_fetch::is_login_wall(html) {
            Post::SignIn
        } else if eddy_fetch::is_bot_wall(html) || html.len() < 20_000 {
            Post::Blocked
        } else {
            Post::Text
        };
    };
    let attrs: Vec<(String, String)> = ATTR
        .captures_iter(tag.as_str())
        .map(|c| (c[1].to_string(), unescape(&c[2])))
        .collect();
    let attr = |name: &str| {
        attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let post_type = attr("post-type").unwrap_or("");
    let href = attr("content-href").unwrap_or("").to_string();

    let items = match post_type {
        "gallery" => gallery(html),
        "video" => VIDEO_ID
            .captures(&href)
            .or_else(|| VIDEO_ID.captures(&html[tag.start()..]))
            .map(|c| {
                vec![hls(
                    &c[1],
                    PLAYER_POSTER.captures(html).map(|p| unescape(&p[1])),
                )]
            })
            .unwrap_or_default(),
        "image" if is_full_size(&href) => vec![MediaItem::image(href.clone())],
        _ => Vec::new(),
    };
    if !items.is_empty() {
        return Post::Media(items);
    }
    if eddy_fetch::is_login_wall(html) && matches!(post_type, "gallery" | "video" | "image") {
        return Post::SignIn;
    }
    if post_type == "link" && !href.is_empty() {
        return Post::Link(href);
    }
    Post::Text
}

/// The pictures of a gallery post, one per carousel page.
fn gallery(html: &str) -> Vec<MediaItem> {
    let Some(start) = html.find("<gallery-carousel") else {
        return Vec::new();
    };
    let end = html[start..]
        .find("</gallery-carousel>")
        .map_or(html.len(), |e| start + e);
    let carousel = &html[start..end];
    let starts: Vec<usize> = PAGE.find_iter(carousel).map(|m| m.start()).collect();
    let mut items = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let page = &carousel[s..starts.get(i + 1).copied().unwrap_or(carousel.len())];
        let Some(src) = FULL_IMAGE
            .captures(page)
            .or_else(|| ANY_IMAGE.captures(page))
            .map(|c| unescape(&c[1]))
        else {
            continue;
        };
        let mut item = MediaItem::image(src);
        if let Some(size) = SIZE.captures(page) {
            item.width = size[1].parse().ok();
            item.height = size[2].parse().ok();
        }
        items.push(item);
    }
    items
}

pub(crate) fn hls(id: &str, poster: Option<String>) -> MediaItem {
    MediaItem {
        poster,
        ..MediaItem::video(format!("https://v.redd.it/{id}/HLSPlaylist.m3u8"))
    }
}

/// Reddit's original files, as opposed to resized previews.
pub(crate) fn is_full_size(src: &str) -> bool {
    src.starts_with("https://i.redd.it/") || src.starts_with("https://v.redd.it/")
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
