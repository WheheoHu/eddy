//! The Gallery: items of marked sources and folders plus saved media links,
//! with their links resolved to playable media on demand (docs/adr/0007).

use std::collections::HashSet;
use std::sync::Arc;

use eddy_media::{MediaSet, MediaStatus, MediaType, PageSource};
use eddy_model::{ContentOrigin, Entity, ItemKind, fields, now_ms};
use eddy_store::{ItemFilter, ItemQuery, ItemRow};

use crate::engine::{Engine, Inner};
use crate::{EngineError, LogLevel, Result};

/// Synced setting: folder (source category) names shown in the Gallery.
pub(crate) const GALLERY_FOLDERS: &str = "gallery.folders";

/// Sites that failed are asked again after this long; ready results are
/// refreshed after a week, since media addresses can expire.
const RETRY_FAILED_MS: i64 = 30 * 60 * 1000;
const RETRY_SIGN_IN_MS: i64 = 10 * 60 * 1000;
const REFRESH_READY_MS: i64 = 7 * 24 * 3600 * 1000;

/// What part of the Gallery to list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GalleryScope {
    All,
    Source(String),
    Folder(String),
    /// Media links saved to the collection.
    Saved,
}

/// A Gallery item with what is known about its media so far.
#[derive(Clone, Debug)]
pub struct GalleryEntry {
    pub row: ItemRow,
    /// Resolved media, when resolved before.
    pub media: Option<MediaSet>,
    /// Grid picture: the resolved thumbnail, else one from the feed content.
    pub thumbnail: Option<String>,
}

pub(crate) struct Pages(pub(crate) Arc<eddy_fetch::Fetcher>);

#[async_trait::async_trait]
impl PageSource for Pages {
    async fn page(&self, url: &str) -> Option<String> {
        self.0
            .fetch(url, |p| {
                !eddy_fetch::is_login_wall(&p.html) && !eddy_fetch::is_bot_wall(&p.html)
            })
            .await
            .ok()
            .map(|p| p.html)
    }
}

pub(crate) fn status_str(status: MediaStatus) -> &'static str {
    match status {
        MediaStatus::Ready => "ready",
        MediaStatus::NoMedia => "no_media",
        MediaStatus::NeedsSignIn => "needs_sign_in",
        MediaStatus::Failed => "failed",
    }
}

impl Inner {
    pub(crate) fn gallery_folders(&self) -> Result<Vec<String>> {
        Ok(self
            .store
            .setting(GALLERY_FOLDERS)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    /// Saved items whose link is media: the collection's Media part.
    fn saved_media_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .store
            .items(&ItemQuery {
                filter: ItemFilter::Saved,
                unread_only: false,
                keep: None,
                limit: 100_000,
                offset: 0,
            })?
            .into_iter()
            .filter(|r| r.item.series_id.is_none())
            .filter(|r| r.item.url.as_deref().is_some_and(eddy_media::is_media_url))
            .map(|r| r.item.id)
            .collect())
    }

    fn gallery_filter(&self, scope: &GalleryScope) -> Result<ItemFilter> {
        let folders: HashSet<String> = self.gallery_folders()?.into_iter().collect();
        let marked = self
            .store
            .sources()?
            .into_iter()
            .filter(|s| s.gallery || s.category.as_ref().is_some_and(|c| folders.contains(c)));
        let (sources, items) = match scope {
            GalleryScope::All => (marked.map(|s| s.id).collect(), self.saved_media_ids()?),
            GalleryScope::Source(id) => (
                marked.filter(|s| &s.id == id).map(|s| s.id).collect(),
                Vec::new(),
            ),
            GalleryScope::Folder(name) => (
                marked
                    .filter(|s| s.category.as_deref() == Some(name))
                    .map(|s| s.id)
                    .collect(),
                Vec::new(),
            ),
            GalleryScope::Saved => (Vec::new(), self.saved_media_ids()?),
        };
        Ok(ItemFilter::Gallery { sources, items })
    }

    /// The content the feed delivered for an item, if still stored.
    fn feed_html(&self, item_id: &str) -> Result<Option<String>> {
        let Some(feed) = self
            .store
            .content_versions(item_id)?
            .into_iter()
            .find(|v| v.origin == ContentOrigin::Feed)
        else {
            return Ok(None);
        };
        Ok(self
            .store
            .get_blob(&feed.raw_blob)?
            .map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    fn cached_media(&self, item_id: &str) -> Result<Option<(MediaSet, i64)>> {
        Ok(self
            .store
            .media_set(item_id)?
            .and_then(|(json, _, at)| serde_json::from_str(&json).ok().map(|s| (s, at))))
    }

    pub(crate) async fn resolve_media(&self, item_id: &str, force: bool) -> Result<MediaSet> {
        if !force && let Some((set, at)) = self.cached_media(item_id)? {
            let age = now_ms() - at;
            let stale = match set.status {
                MediaStatus::Ready => age > REFRESH_READY_MS,
                MediaStatus::NoMedia => false,
                MediaStatus::NeedsSignIn => age > RETRY_SIGN_IN_MS,
                MediaStatus::Failed => age > RETRY_FAILED_MS,
            };
            if !stale {
                return Ok(set);
            }
        }
        let _permit = self
            .media_gate
            .acquire()
            .await
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        let row = self.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let url = row.item.url.clone().unwrap_or_default();
        let feed = self.feed_html(item_id)?;
        let pages = Pages(self.fetcher.read().clone());
        let set = self.media.resolve(&url, feed.as_deref(), &pages).await;
        self.store.put_media_set(
            item_id,
            &serde_json::to_string(&set)?,
            status_str(set.status),
        )?;
        Ok(set)
    }

    /// Whether `url` is gone for good: the site says so (404, 410) or sends
    /// its "removed" picture. Anything else, including no answer, is not.
    async fn gone(&self, url: &str) -> bool {
        let head = self.http.head(url).send().await;
        let response = match head {
            Ok(r) if r.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                self.http.get(url).send().await
            }
            other => other,
        };
        let Ok(response) = response else {
            return false;
        };
        matches!(response.status().as_u16(), 404 | 410) || is_removed_placeholder(response.url())
    }

    /// Drops the media of a set that the sites have removed and replaces a
    /// removed thumbnail; with nothing left the item has no media and
    /// leaves the Gallery.
    pub(crate) async fn verify_media(&self, item_id: &str) -> Result<Option<MediaSet>> {
        let Some((mut set, _)) = self.cached_media(item_id)? else {
            return Ok(None);
        };
        let mut kept = Vec::with_capacity(set.items.len());
        for item in std::mem::take(&mut set.items) {
            // Players load their own pages; only files can be checked.
            if item.kind == MediaType::Embed || !self.gone(&item.src).await {
                kept.push(item);
            }
        }
        set.items = kept;
        if set.items.is_empty() {
            set.status = MediaStatus::NoMedia;
            set.thumbnail = None;
        } else if let Some(thumbnail) = set.thumbnail.clone()
            && self.gone(&thumbnail).await
        {
            set.thumbnail = set
                .items
                .iter()
                .find_map(|m| m.preview().map(String::from));
        }
        self.store.put_media_set(
            item_id,
            &serde_json::to_string(&set)?,
            status_str(set.status),
        )?;
        Ok(Some(set))
    }
}

/// Image hosts answer a removed file with a stand-in picture.
fn is_removed_placeholder(url: &url::Url) -> bool {
    url.path().ends_with("/removed.png")
}

impl Engine {
    /// Called when an item's thumbnail fails to load: checks its media
    /// with the sites and drops what was removed. Returns the updated set.
    pub async fn verify_media(&self, item_id: &str) -> Result<Option<MediaSet>> {
        self.inner.verify_media(item_id).await
    }

    /// Gallery items, newest first.
    pub fn gallery_items(
        &self,
        scope: &GalleryScope,
        unread_only: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<GalleryEntry>> {
        let inner = &self.inner;
        let rows = inner.store.items(&ItemQuery {
            filter: inner.gallery_filter(scope)?,
            unread_only,
            keep: None,
            limit,
            offset,
        })?;
        rows.into_iter()
            .map(|row| {
                let media = inner.cached_media(&row.item.id)?.map(|(s, _)| s);
                let thumbnail = match media.as_ref().and_then(|m| m.thumbnail.clone()) {
                    Some(t) => Some(t),
                    None => inner
                        .feed_html(&row.item.id)?
                        .and_then(|html| eddy_media::feed_thumbnail(&html)),
                };
                Ok(GalleryEntry {
                    row,
                    media,
                    thumbnail,
                })
            })
            .collect()
    }

    pub fn gallery_count(&self, scope: &GalleryScope, unread_only: bool) -> Result<u32> {
        let inner = &self.inner;
        Ok(inner.store.count_items(&ItemQuery {
            filter: inner.gallery_filter(scope)?,
            unread_only,
            keep: None,
            limit: 0,
            offset: 0,
        })?)
    }

    /// The item's media, resolved now unless a fresh result is stored.
    /// `force` asks again, for example after signing in.
    pub async fn resolve_media(&self, item_id: &str, force: bool) -> Result<MediaSet> {
        self.inner.resolve_media(item_id, force).await
    }

    /// Resolves items in the background, a few at a time, so the grid has
    /// them by the time they scroll into view.
    pub fn prefetch_media(&self, item_ids: Vec<String>) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            futures::stream::iter(item_ids)
                .for_each_concurrent(4, |id| {
                    let inner = inner.clone();
                    async move {
                        if !inner.media_pending.lock().insert(id.clone()) {
                            return;
                        }
                        if let Err(e) = inner.resolve_media(&id, false).await {
                            inner
                                .platform
                                .log(LogLevel::Info, &format!("media prefetch skipped: {e}"));
                        }
                        inner.media_pending.lock().remove(&id);
                    }
                })
                .await;
        });
    }

    pub fn set_source_gallery(&self, source_id: &str, shown: bool) -> Result<()> {
        self.inner.record(
            Entity::Source,
            source_id,
            fields::GALLERY,
            shown.into(),
            false,
        )
    }

    pub fn gallery_folders(&self) -> Result<Vec<String>> {
        self.inner.gallery_folders()
    }

    pub fn set_folder_gallery(&self, name: &str, shown: bool) -> Result<()> {
        let mut folders = self.inner.gallery_folders()?;
        folders.retain(|f| f != name);
        if shown {
            folders.push(name.to_string());
        }
        folders.sort();
        self.inner.record(
            Entity::Setting,
            GALLERY_FOLDERS,
            fields::VALUE,
            serde_json::to_value(folders)?,
            false,
        )
    }

    /// Whether an item belongs in the Gallery (marked source or folder, or a
    /// saved media link).
    pub fn in_gallery(&self, row: &ItemRow) -> Result<bool> {
        if row.item.kind == ItemKind::Saved {
            return Ok(row
                .item
                .url
                .as_deref()
                .is_some_and(eddy_media::is_media_url));
        }
        let Some(source) = row
            .item
            .source_id
            .as_deref()
            .and_then(|s| self.inner.store.source(s).ok().flatten())
        else {
            return Ok(false);
        };
        Ok(source.gallery
            || source
                .category
                .as_ref()
                .is_some_and(|c| self.inner.gallery_folders().is_ok_and(|f| f.contains(c))))
    }
}
