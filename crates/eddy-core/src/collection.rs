//! The collection: everything that is not a subscription article, in one
//! shape. A series (an imported book, a series from a table of contents) is
//! one entry; so is a page or PDF saved on its own, as a single chapter.
//! Storage keeps its own model; this is a view over it.

use std::collections::HashSet;

use eddy_model::{ContentFormat, Entity, ItemKind, fields};
use eddy_store::{ItemFilter, ItemQuery, ItemRow};

use crate::engine::{Engine, Inner};
use crate::progress::SeriesInfo;
use crate::{EngineError, LogLevel, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    /// An imported EPUB.
    Book,
    /// An imported PDF.
    Pdf,
    /// A page saved on its own.
    WebPage,
    /// Chapters collected from a table of contents.
    WebSeries,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollectionEntry {
    /// The series ID, or the item ID of a page or PDF saved on its own.
    pub id: String,
    pub kind: CollectionKind,
    pub series_id: Option<String>,
    pub title: String,
    pub author: Option<String>,
    /// A book's own cover image, for `Engine::image`.
    pub cover: Option<String>,
    /// The page, or a series' table of contents.
    pub url: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    /// Overall progress, 0..=1.
    pub progress: f64,
    /// The chapter to open when the entry is opened.
    pub resume_item_id: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
    pub last_read_at: Option<i64>,
    pub text_len: u32,
    pub lang: Option<String>,
}

impl CollectionEntry {
    fn from_series(info: SeriesInfo) -> Self {
        CollectionEntry {
            kind: if info.toc_url.is_some() {
                CollectionKind::WebSeries
            } else {
                CollectionKind::Book
            },
            series_id: Some(info.id.clone()),
            id: info.id,
            title: info.title,
            author: info.author,
            cover: info.cover,
            url: info.toc_url,
            chapters: info.chapters,
            finished: info.finished,
            progress: info.progress,
            resume_item_id: info.resume_item_id,
            added_at: info.created_at,
            updated_at: info.updated_at,
            last_read_at: info.last_read_at,
            text_len: info.text_len,
            lang: info.lang,
        }
    }

    fn from_item(row: ItemRow) -> Self {
        let item = row.item;
        CollectionEntry {
            kind: if row.format == ContentFormat::Pdf {
                CollectionKind::Pdf
            } else {
                CollectionKind::WebPage
            },
            series_id: None,
            resume_item_id: Some(item.id.clone()),
            id: item.id,
            title: item.title,
            author: item.author,
            cover: None,
            url: item.url,
            chapters: 1,
            finished: item.finished as u32,
            progress: if item.finished { 1.0 } else { item.progress },
            added_at: item.added_at,
            updated_at: item.published_at,
            last_read_at: item.last_read_at,
            text_len: item.text_len,
            lang: item.lang,
        }
    }
}

impl Engine {
    /// Every series and every page or PDF saved on its own. Pages that
    /// joined a series, and the table of contents it was made from, appear
    /// only as the series.
    pub fn collection(&self) -> Result<Vec<CollectionEntry>> {
        let inner = &self.inner;
        let mut entries = Vec::new();
        let mut series_ids = HashSet::new();
        let mut contents_pages = HashSet::new();
        for (series, _, _) in inner.store.series_list()? {
            series_ids.insert(series.id.clone());
            contents_pages.extend(series.toc_url.clone());
            entries.push(CollectionEntry::from_series(inner.series_info(series)?));
        }
        let saved = inner.store.items(&ItemQuery {
            filter: ItemFilter::Saved,
            unread_only: false,
            keep: None,
            limit: 100_000,
            offset: 0,
        })?;
        entries.extend(
            saved
                .into_iter()
                .filter(|r| {
                    r.item
                        .series_id
                        .as_ref()
                        .is_none_or(|s| !series_ids.contains(s))
                        && r.item
                            .url
                            .as_ref()
                            .is_none_or(|u| !contents_pages.contains(u))
                })
                .map(CollectionEntry::from_item),
        );
        Ok(entries)
    }

    /// Deletes a collection entry everywhere: a series with all its
    /// chapters and its saved table of contents, or a single saved page or
    /// PDF. Subscription articles belong to their feed and cannot be deleted.
    pub async fn delete_entry(&self, id: &str) -> Result<()> {
        self.inner.delete_entry(id)?;
        self.inner.push_in_background();
        Ok(())
    }
}

impl Inner {
    pub(crate) fn delete_entry(&self, id: &str) -> Result<()> {
        let items: Vec<String> = if let Some(series) = self.store.series(id)? {
            self.record(Entity::Series, id, fields::DELETED, true.into(), false)?;
            let mut items: Vec<String> =
                self.chapters(id)?.into_iter().map(|r| r.item.id).collect();
            if let Some(url) = &series.toc_url
                && let Some(contents) = self.saved_by_url(url)?
                && !items.contains(&contents.item.id)
            {
                items.push(contents.item.id);
            }
            items
        } else {
            let row = self.store.item(id)?.ok_or(EngineError::NotFound)?;
            if row.item.kind == ItemKind::Feed {
                return Err(EngineError::Invalid("订阅文章不能删除。".into()));
            }
            vec![row.item.id]
        };
        for item_id in &items {
            self.record(Entity::Item, item_id, fields::DELETED, true.into(), false)?;
        }
        self.forget_deleted(&items);
        Ok(())
    }

    /// Drops deleted items from the search index and frees the files only
    /// they used. Runs for local deletions and ones synced from elsewhere.
    pub(crate) fn forget_deleted(&self, item_ids: &[String]) {
        if item_ids.is_empty() {
            return;
        }
        for item_id in item_ids {
            self.unindex_item(item_id);
        }
        self.commit_index();
        if let Err(e) = self.store.purge_orphan_blobs() {
            self.platform.log(
                LogLevel::Warn,
                &format!("removing deleted files failed: {e}"),
            );
        }
    }
}
