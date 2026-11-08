use scraper::{Html, Selector};

use crate::{MediaItem, MediaType};

/// Images and videos in the content a feed delivered, in document order.
/// A reduced preview of a picture that is also there in full is dropped.
pub fn feed_media(html: &str) -> Vec<MediaItem> {
    let doc = Html::parse_fragment(html);
    let selector = Selector::parse("img, video, video source").unwrap();
    let mut items: Vec<MediaItem> = Vec::new();
    for el in doc.select(&selector) {
        let attr = |name: &str| {
            el.value()
                .attr(name)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let dimension = |name: &str| attr(name).and_then(|v| v.parse::<u32>().ok());
        let Some(src) = attr("src")
            .or_else(|| attr("data-src"))
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        else {
            continue;
        };
        if dimension("width").is_some_and(|w| w <= 2) || dimension("height").is_some_and(|h| h <= 2)
        {
            continue;
        }
        let mut item = match el.value().name() {
            "img" => MediaItem::image(src),
            _ => MediaItem {
                poster: el
                    .value()
                    .attr("poster")
                    .or_else(|| {
                        el.parent()
                            .and_then(|p| p.value().as_element())
                            .and_then(|p| p.attr("poster"))
                    })
                    .map(String::from),
                ..MediaItem::video(src)
            },
        };
        item.width = dimension("width");
        item.height = dimension("height");
        if !items.iter().any(|m| m.src == item.src) {
            items.push(item);
        }
    }
    let full: Vec<String> = items
        .iter()
        .filter(|m| m.kind == MediaType::Image && !is_preview(&m.src))
        .map(|m| stem(&m.src))
        .filter(|s| !s.is_empty())
        .collect();
    items.retain(|m| {
        !(is_preview(&m.src) && full.iter().any(|f| stem(&m.src).ends_with(f.as_str())))
    });
    items
}

/// The grid picture the feed content offers, before anything is resolved.
pub fn feed_thumbnail(html: &str) -> Option<String> {
    thumbnail(&feed_media(html))
}

/// The smallest ready-made picture among the feed's media.
pub(crate) fn thumbnail(items: &[MediaItem]) -> Option<String> {
    items
        .iter()
        .find(|m| m.kind == MediaType::Image && is_preview(&m.src))
        .or_else(|| items.iter().find(|m| m.preview().is_some()))
        .and_then(|m| m.preview().map(String::from))
}

fn is_preview(src: &str) -> bool {
    url::Url::parse(src)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.contains("preview") || h.contains("thumb"))
        })
        .unwrap_or(false)
}

fn stem(src: &str) -> String {
    url::Url::parse(src)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(String::from))
        })
        .map(|name| name.split('.').next().unwrap_or("").to_string())
        .unwrap_or_default()
}
