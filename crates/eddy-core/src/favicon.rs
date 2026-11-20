//! A feed's icon: the favicon of its site. It is looked for on this device,
//! from the site's address the feed already carries, kept in the blob cache
//! and remembered in a local table, so it shows offline and nothing about it
//! is synced. A site without one is remembered too, for a week.

use eddy_model::ids::sha256_hex;
use eddy_model::now_ms;

use crate::engine::{Engine, Inner};
use crate::{LogLevel, Result};

/// A site found to have no icon is asked again after this long.
const RECHECK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// A page's head is at its start; a megabyte of it is plenty.
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const MAX_ICON_BYTES: usize = 1024 * 1024;
/// Icons a page names, tried in order before giving up on it.
const MAX_CANDIDATES: usize = 4;

/// What looking for a site's icon came to.
enum Lookup {
    Found(Vec<u8>, &'static str),
    /// The site answered and has none that can be shown.
    Missing,
    /// Nothing could be asked: offline, or the site is down. Not remembered.
    Unreachable,
}

impl Engine {
    /// The icon of a subscription's site and its MIME type; `None` when the
    /// site has none, or it could not be looked for just now. Never an error
    /// for anything the network did.
    pub async fn feed_icon(&self, source_id: &str) -> Result<Option<(Vec<u8>, String)>> {
        self.inner.feed_icon(source_id).await
    }
}

impl Inner {
    pub(crate) async fn feed_icon(&self, source_id: &str) -> Result<Option<(Vec<u8>, String)>> {
        if let Some(known) = self.kept_icon(source_id)? {
            return Ok(known);
        }
        let site = self
            .store
            .source(source_id)?
            .and_then(|s| s.site_url)
            .and_then(|u| url::Url::parse(&u).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        let Some(site) = site else {
            return Ok(None);
        };
        // A sidebar asks for every feed at once; a few sites at a time.
        let Ok(_turn) = self.icon_gate.acquire().await else {
            return Ok(None);
        };
        // Found while this one waited its turn.
        if let Some(known) = self.kept_icon(source_id)? {
            return Ok(known);
        }
        match self.find_icon(&site).await {
            Lookup::Found(bytes, mime) => {
                let key = format!("ico{}", &sha256_hex(source_id.as_bytes())[..40]);
                // Not pinned: evicted with the rest of the cache, it is
                // looked for again.
                self.store.put_blob_keyed(&key, &bytes, mime, false)?;
                self.store
                    .set_source_icon(source_id, Some(&key), now_ms())?;
                Ok(Some((bytes, mime.to_string())))
            }
            Lookup::Missing => {
                self.store.set_source_icon(source_id, None, now_ms())?;
                Ok(None)
            }
            Lookup::Unreachable => Ok(None),
        }
    }

    /// What is already known: `Some(Some(icon))`, `Some(None)` for a site
    /// that had none when last asked, `None` when it is time to look.
    #[allow(clippy::type_complexity)]
    fn kept_icon(&self, source_id: &str) -> Result<Option<Option<(Vec<u8>, String)>>> {
        let Some((key, checked)) = self.store.source_icon(source_id)? else {
            return Ok(None);
        };
        match key {
            Some(key) => match self.store.get_blob(&key)? {
                Some(bytes) => {
                    let mime = self
                        .store
                        .blob_mime(&key)?
                        .unwrap_or_else(|| "image/*".into());
                    Ok(Some(Some((bytes, mime))))
                }
                // Evicted from the cache.
                None => Ok(None),
            },
            None if now_ms() - checked < RECHECK_MS => Ok(Some(None)),
            None => Ok(None),
        }
    }

    /// The icons the site's page names, in order, then `/favicon.ico`. The
    /// address a feed gives for its site is sometimes the feed itself, or a
    /// page deep in the site that names no icon: the site's front page is
    /// read then.
    async fn find_icon(&self, site: &url::Url) -> Lookup {
        let (mut reached, mut candidates) = self.icons_named_at(site).await;
        if candidates.is_empty()
            && let Ok(front) = site.join("/")
            && front != *site
        {
            let (front_reached, named) = self.icons_named_at(&front).await;
            reached |= front_reached;
            candidates = named;
        }
        if let Ok(fallback) = site.join("/favicon.ico") {
            let fallback = String::from(fallback);
            if !candidates.contains(&fallback) {
                candidates.push(fallback);
            }
        }
        for candidate in candidates {
            let Ok(response) = self.http.get(&candidate).timeout(TIMEOUT).send().await else {
                continue;
            };
            reached = true;
            if !response.status().is_success()
                || response
                    .content_length()
                    .is_some_and(|n| n as usize > MAX_ICON_BYTES)
            {
                continue;
            }
            let Ok(bytes) = response.bytes().await else {
                continue;
            };
            // Sites answer a missing favicon with a page as often as with 404.
            if bytes.len() <= MAX_ICON_BYTES
                && let Some(mime) = image_kind(&bytes)
            {
                return Lookup::Found(bytes.to_vec(), mime);
            }
        }
        if reached {
            Lookup::Missing
        } else {
            Lookup::Unreachable
        }
    }
}

impl Inner {
    /// Whether the page answered at all, and the icons it names.
    async fn icons_named_at(&self, page: &url::Url) -> (bool, Vec<String>) {
        let response = match self.http.get(page.clone()).timeout(TIMEOUT).send().await {
            Ok(response) => response,
            Err(e) => {
                self.platform.log(
                    LogLevel::Info,
                    &format!("icon: {} not reached: {}", host(page), e.without_url()),
                );
                return (false, Vec::new());
            }
        };
        let base = response.url().to_string();
        let fits = response
            .content_length()
            .is_none_or(|n| n as usize <= MAX_PAGE_BYTES);
        if !response.status().is_success() || !fits {
            return (true, Vec::new());
        }
        let Ok(body) = response.bytes().await else {
            return (true, Vec::new());
        };
        let head = &body[..body.len().min(MAX_PAGE_BYTES)];
        let mut named = eddy_extract::icon_links(&String::from_utf8_lossy(head), &base);
        named.truncate(MAX_CANDIDATES);
        (true, named)
    }
}

fn host(url: &url::Url) -> &str {
    url.host_str().unwrap_or_default()
}

/// The kind of image the bytes are, by how they begin; `None` for anything
/// else, whatever the server called it.
fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some("image/x-icon")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        let start = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_lowercase();
        let start = start.trim_start_matches('\u{feff}').trim_start();
        let svg = start.starts_with("<svg")
            || ((start.starts_with("<?xml") || start.starts_with("<!--"))
                && start.contains("<svg"));
        svg.then_some("image/svg+xml")
    }
}

#[cfg(test)]
mod tests {
    use super::image_kind;

    #[test]
    fn images_are_told_by_how_they_begin() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(image_kind(&[0, 0, 1, 0, 1, 0]), Some("image/x-icon"));
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_kind(b"GIF89a.."), Some("image/gif"));
        assert_eq!(image_kind(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(
            image_kind(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(
            image_kind(b"<?xml version=\"1.0\"?>\n<svg viewBox=\"0 0 1 1\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(image_kind(b"<!doctype html><html><body>Not Found"), None);
        assert_eq!(image_kind(b"<?xml version=\"1.0\"?><rss/>"), None);
        assert_eq!(image_kind(b""), None);
    }
}
