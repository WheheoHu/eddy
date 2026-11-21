//! The results of requests: the shapes tools and agents read as JSON.
//!
//! They are separate from the engine's own types on purpose. Those change
//! with the engine, and `Item` in particular is written the way the sync
//! protocol needs it; these only gain fields. Every result is an object, so
//! that stays possible. Times are milliseconds since 1970; pages count
//! from 1.

use std::collections::BTreeMap;

use eddy_core::{
    CollectionEntry, CollectionKind, ItemRow, ResumePoint, SeriesInfo, SourceInfo, SourceStats,
};
use eddy_model::{ContentFormat, ItemKind, Position};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemDto {
    pub id: String,
    /// `feed` (a subscription's article), `saved` or `chapter`.
    pub kind: String,
    /// `article` or `pdf`.
    pub format: String,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    /// The subscription a `feed` item came from, and its title.
    pub feed_id: Option<String>,
    pub feed: Option<String>,
    pub series_id: Option<String>,
    pub chapter_index: Option<u32>,
    pub published_at: i64,
    pub added_at: i64,
    pub read: bool,
    pub starred: bool,
    pub finished: bool,
    /// How far it has been read, 0 to 1.
    pub progress: f64,
    pub last_read_at: Option<i64>,
    /// Characters of text.
    pub text_len: u32,
    pub lang: Option<String>,
    pub excerpt: Option<String>,
    /// Tag names; only where one item is asked for by ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl From<&ItemRow> for ItemDto {
    fn from(row: &ItemRow) -> Self {
        let item = &row.item;
        // Saved pages and chapters have a source of their own inside the
        // engine; it is not a subscription anyone can list or filter by.
        let subscribed = item.kind == ItemKind::Feed;
        ItemDto {
            id: item.id.clone(),
            kind: item.kind.as_str().to_string(),
            format: match row.format {
                ContentFormat::Article => "article",
                ContentFormat::Pdf => "pdf",
            }
            .to_string(),
            title: item.title.clone(),
            url: item.url.clone(),
            author: item.author.clone(),
            feed_id: subscribed.then(|| item.source_id.clone()).flatten(),
            feed: subscribed.then(|| row.source_title.clone()).flatten(),
            series_id: item.series_id.clone(),
            chapter_index: item.chapter_index,
            published_at: item.published_at,
            added_at: item.added_at,
            read: item.read,
            starred: item.starred,
            finished: item.finished,
            progress: item.progress,
            last_read_at: item.last_read_at,
            text_len: item.text_len,
            lang: item.lang.clone(),
            excerpt: item.excerpt.clone(),
            tags: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemPage {
    pub items: Vec<ItemDto>,
    /// How many items the query matches in all.
    pub total: u32,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Items {
    pub items: Vec<ItemDto>,
}

/// How many items a request changed, or with `dry_run` would have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTags {
    pub id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocDto {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub format: String,
    /// Pages of a PDF; absent for anything else.
    pub pages: Option<u32>,
    pub text_len: u32,
    /// The text, in every format but `json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The document model, for `json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedDto {
    /// What to open: the page, the PDF, or a book's first chapter.
    pub id: String,
    pub title: String,
    /// It was there already; nothing new was made.
    pub existing: bool,
    /// The series a book was imported as.
    pub series_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<ChapterDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterDto {
    pub id: String,
    pub title: String,
    pub text_len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedDto {
    pub id: String,
    pub title: String,
    pub folder: Option<String>,
    pub site_url: Option<String>,
    pub unread: u32,
    pub total: u32,
    pub latest_published_at: Option<i64>,
}

impl FeedDto {
    pub fn new(info: &SourceInfo, stats: Option<&SourceStats>) -> Self {
        FeedDto {
            id: info.source.id.clone(),
            title: info.source.title.clone(),
            folder: info.source.category.clone(),
            site_url: info.source.site_url.clone(),
            unread: info.unread,
            total: stats.map_or(0, |s| s.total),
            latest_published_at: stats.and_then(|s| s.latest_published_at),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feeds {
    pub feeds: Vec<FeedDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedAdded {
    pub id: String,
    pub title: String,
    pub folder: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderDto {
    pub name: String,
    pub feeds: u32,
    pub unread: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folders {
    pub folders: Vec<FolderDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHitDto {
    pub item: ItemDto,
    /// The text around the match.
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHits {
    pub hits: Vec<SearchHitDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgressDto {
    pub id: String,
    /// 0 to 1.
    pub progress: f64,
    pub finished: bool,
    pub last_read_at: Option<i64>,
    /// Where reading stopped: the block and the UTF-16 offset in it.
    pub block: Option<String>,
    pub offset: Option<u32>,
    /// The page of a PDF.
    pub page: Option<u32>,
    /// The words at that position.
    pub quote: Option<String>,
    /// The position was recorded on another device.
    pub from_other_device: bool,
}

impl ProgressDto {
    pub fn new(row: &ItemRow, resume: Option<&ResumePoint>) -> Self {
        let page = resume.and_then(|r| match &r.locator.position {
            Position::Pdf { page, .. } => Some(page + 1),
            _ => r.page.map(|p| p + 1),
        });
        ProgressDto {
            id: row.item.id.clone(),
            progress: row.item.progress,
            finished: row.item.finished,
            last_read_at: row.item.last_read_at,
            block: resume.and_then(|r| r.block_id.clone()),
            offset: resume.filter(|r| r.block_id.is_some()).map(|r| r.offset),
            page,
            quote: resume
                .map(|r| r.locator.quote.exact.clone())
                .filter(|q| !q.is_empty()),
            from_other_device: resume.is_some_and(|r| r.from_other_device),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDto {
    pub id: String,
    pub name: String,
    /// Items carrying it.
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tags {
    pub tags: Vec<TagDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesDto {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    /// The table of contents it was collected from.
    pub url: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    pub progress: f64,
    /// The chapter to continue with.
    pub next_item_id: Option<String>,
    pub created_at: i64,
    pub last_read_at: Option<i64>,
    pub text_len: u32,
}

impl From<&SeriesInfo> for SeriesDto {
    fn from(info: &SeriesInfo) -> Self {
        SeriesDto {
            id: info.id.clone(),
            title: info.title.clone(),
            author: info.author.clone(),
            url: info.toc_url.clone(),
            chapters: info.chapters,
            finished: info.finished,
            progress: info.progress,
            next_item_id: info.resume_item_id.clone(),
            created_at: info.created_at,
            last_read_at: info.last_read_at,
            text_len: info.text_len,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesList {
    pub series: Vec<SeriesDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesDetail {
    pub series: SeriesDto,
    pub chapters: Vec<ItemDto>,
}

/// A table of contents found on a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesFound {
    pub found: bool,
    pub title: Option<String>,
    pub url: Option<String>,
    pub chapters: Vec<ChapterLinkDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterLinkDto {
    pub title: String,
    pub url: String,
    pub level: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesCreated {
    pub id: String,
    pub title: String,
    pub chapters: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollectionEntryDto {
    /// A series ID, or the item ID of a page or PDF saved on its own.
    pub id: String,
    /// `book`, `pdf`, `web_page`, `media` or `web_series`.
    pub kind: String,
    pub series_id: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub url: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    pub progress: f64,
    /// The item to open.
    pub next_item_id: Option<String>,
    pub added_at: i64,
    pub last_read_at: Option<i64>,
    pub text_len: u32,
    pub lang: Option<String>,
}

pub fn collection_kind(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::Book => "book",
        CollectionKind::Pdf => "pdf",
        CollectionKind::WebPage => "web_page",
        CollectionKind::Media => "media",
        CollectionKind::WebSeries => "web_series",
    }
}

impl From<&CollectionEntry> for CollectionEntryDto {
    fn from(entry: &CollectionEntry) -> Self {
        CollectionEntryDto {
            id: entry.id.clone(),
            kind: collection_kind(entry.kind).to_string(),
            series_id: entry.series_id.clone(),
            title: entry.title.clone(),
            author: entry.author.clone(),
            url: entry.url.clone(),
            chapters: entry.chapters,
            finished: entry.finished,
            progress: entry.progress,
            next_item_id: entry.resume_item_id.clone(),
            added_at: entry.added_at,
            last_read_at: entry.last_read_at,
            text_len: entry.text_len,
            lang: entry.lang.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub entries: Vec<CollectionEntryDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryDto {
    pub id: String,
    /// One sentence; absent when there is no summary yet.
    pub tldr: Option<String>,
    pub points: Vec<String>,
    /// It was there already; the model was not asked.
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationDto {
    pub id: String,
    /// Absent when the item has not been translated.
    pub language: Option<String>,
    pub complete: bool,
    /// Block ID (or `#caption`, `#rXcY` for captions and table cells) to
    /// its translation.
    pub segments: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exported {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusDto {
    /// `unconfigured`, `standalone` (FreshRSS directly) or `server`.
    pub mode: String,
    pub account: Option<AccountDto>,
    pub last_refresh: Option<i64>,
    pub last_error: Option<String>,
    /// Only in server mode.
    pub server: Option<ServerDto>,
    pub core: String,
    pub sync_protocol: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDto {
    pub url: String,
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDto {
    pub online: bool,
    pub url: Option<String>,
    pub version: Option<String>,
    pub cursor: Option<i64>,
    /// Why the server could not be asked just now.
    pub error: Option<String>,
    /// Changes waiting to be uploaded.
    pub pending_uploads: u32,
    /// Changes the server refused for good.
    pub rejected_uploads: u32,
    /// New articles still arrive from FreshRSS while the server is offline.
    pub offline_refresh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncDto {
    pub new_items: u32,
    pub state_changes: u32,
    /// The server was unreachable: new articles came straight from FreshRSS.
    pub offline: bool,
    /// Older articles fetched while filling in the history.
    pub history: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloDto {
    pub control: u32,
    pub core: String,
    pub pid: u32,
}
