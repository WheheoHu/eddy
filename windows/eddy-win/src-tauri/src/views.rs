//! serde views over engine types for the webview. Core types that already
//! derive `Serialize` (Item, Source, Document, Account, AiSettings…) are
//! embedded directly; the rest get small mirrors here. Field names stay
//! snake_case everywhere to match the core's serde attributes.

use eddy_core::{EngineStatus, Imported, ItemFilter, ItemQuery, ItemRow, Mode, RefreshSummary,
                SearchResult, ServerStatus, SourceInfo, SourceStats, SyncPhase, SyncProgress};
use eddy_model::{ContentVersion, Document, Item};
use serde::{Deserialize, Serialize};

// ---------- status ----------

#[derive(Serialize)]
pub struct StatusView {
    pub mode: &'static str,
    pub account: Option<eddy_model::Account>,
    pub last_refresh: Option<i64>,
    pub last_error: Option<String>,
    pub server_online: bool,
    pub pending_uploads: u32,
    pub rejected_uploads: u32,
    pub offline_refresh: bool,
    pub sync: Option<SyncProgressView>,
}

impl From<EngineStatus> for StatusView {
    fn from(s: EngineStatus) -> Self {
        StatusView {
            mode: match s.mode {
                Mode::Unconfigured => "unconfigured",
                Mode::Standalone => "standalone",
                Mode::Server => "server",
            },
            account: s.account,
            last_refresh: s.last_refresh,
            last_error: s.last_error,
            server_online: s.server_online,
            pending_uploads: s.pending_uploads,
            rejected_uploads: s.rejected_uploads,
            offline_refresh: s.offline_refresh,
            sync: s.sync.map(SyncProgressView::from),
        }
    }
}

#[derive(Serialize)]
pub struct SyncProgressView {
    pub phase: &'static str,
    pub done: u64,
    pub total: Option<u64>,
}

impl From<SyncProgress> for SyncProgressView {
    fn from(p: SyncProgress) -> Self {
        SyncProgressView {
            phase: match p.phase {
                SyncPhase::Uploading => "uploading",
                SyncPhase::Downloading => "downloading",
                SyncPhase::FetchingContent => "fetching_content",
            },
            done: p.done,
            total: p.total,
        }
    }
}

#[derive(Serialize)]
pub struct RefreshSummaryView {
    pub new_items: u32,
    pub state_changes: u32,
    pub sources: u32,
    pub new_item_ids: Vec<String>,
    pub offline: bool,
}

impl From<RefreshSummary> for RefreshSummaryView {
    fn from(r: RefreshSummary) -> Self {
        RefreshSummaryView {
            new_items: r.new_items,
            state_changes: r.state_changes,
            sources: r.sources,
            new_item_ids: r.new_item_ids,
            offline: r.offline,
        }
    }
}

// ---------- lists ----------

#[derive(Serialize)]
pub struct ItemRowView {
    pub item: Item,
    pub source_title: Option<String>,
    pub format: &'static str,
}

impl From<ItemRow> for ItemRowView {
    fn from(r: ItemRow) -> Self {
        ItemRowView {
            item: r.item,
            source_title: r.source_title,
            format: match r.format {
                eddy_model::ContentFormat::Article => "article",
                eddy_model::ContentFormat::Pdf => "pdf",
            },
        }
    }
}

#[derive(Serialize)]
pub struct SourceView {
    pub source: eddy_model::Source,
    pub unread: u32,
}

impl From<SourceInfo> for SourceView {
    fn from(s: SourceInfo) -> Self {
        SourceView {
            source: s.source,
            unread: s.unread,
        }
    }
}

#[derive(Serialize)]
pub struct SourceStatsView {
    pub source_id: String,
    pub total: u32,
    pub unread: u32,
}

impl From<SourceStats> for SourceStatsView {
    fn from(s: SourceStats) -> Self {
        SourceStatsView {
            source_id: s.source_id,
            total: s.total,
            unread: s.unread,
        }
    }
}

#[derive(Serialize)]
pub struct SearchResultView {
    pub item: ItemRowView,
    pub snippet: String,
    pub highlights: Vec<(u32, u32)>,
}

impl From<SearchResult> for SearchResultView {
    fn from(r: SearchResult) -> Self {
        SearchResultView {
            item: r.item.into(),
            snippet: r.snippet,
            highlights: r.highlights,
        }
    }
}

// ---------- documents ----------

#[derive(Serialize)]
pub struct DocumentView {
    pub version: ContentVersion,
    pub document: Document,
}

impl From<eddy_core::LoadedDocument> for DocumentView {
    fn from(d: eddy_core::LoadedDocument) -> Self {
        DocumentView {
            version: d.version,
            document: d.document,
        }
    }
}

#[derive(Serialize)]
pub struct ImportedView {
    pub item_id: String,
    pub series_id: Option<String>,
    pub title: String,
    pub existing: bool,
}

impl From<Imported> for ImportedView {
    fn from(i: Imported) -> Self {
        ImportedView {
            item_id: i.item_id,
            series_id: i.series_id,
            title: i.title,
            existing: i.existing,
        }
    }
}

// ---------- reading position ----------

#[derive(Serialize)]
pub struct ResumePointView {
    pub block_id: Option<String>,
    pub offset: u32,
    pub page: Option<u32>,
    pub progression: f64,
    pub from_other_device: bool,
    pub local_block_id: Option<String>,
    pub local_offset: u32,
}

impl From<eddy_core::ResumePoint> for ResumePointView {
    fn from(r: eddy_core::ResumePoint) -> Self {
        ResumePointView {
            block_id: r.block_id,
            offset: r.offset,
            page: r.page,
            progression: r.locator.progression,
            from_other_device: r.from_other_device,
            local_block_id: r.local_block_id,
            local_offset: r.local_offset,
        }
    }
}

// ---------- server ----------

#[derive(Serialize)]
pub struct ServerStatusView {
    pub url: String,
    pub server_id: String,
    pub cursor: i64,
    pub video_capable: bool,
    pub version: String,
}

impl From<ServerStatus> for ServerStatusView {
    fn from(s: ServerStatus) -> Self {
        ServerStatusView {
            url: s.url,
            server_id: s.server_id,
            cursor: s.cursor,
            video_capable: s.video_capable,
            version: s.version,
        }
    }
}

// ---------- AI ----------

#[derive(Serialize)]
pub struct TagView {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
pub struct TranslationBatchView {
    pub item_id: String,
    pub segments: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
pub struct TranslationView {
    pub content_hash: String,
    pub target_language: String,
    pub segments: std::collections::HashMap<String, String>,
    pub cached: bool,
    pub complete: bool,
}

impl From<eddy_core::TranslationResult> for TranslationView {
    fn from(t: eddy_core::TranslationResult) -> Self {
        TranslationView {
            content_hash: t.content_hash,
            target_language: t.target_language,
            segments: t.segments,
            cached: t.cached,
            complete: t.complete,
        }
    }
}

#[derive(Serialize)]
pub struct SummaryView {
    pub tldr: String,
    pub points: Vec<String>,
    pub cached: bool,
}

// ---------- query (frontend → core) ----------

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterView {
    All,
    Starred,
    Saved,
    ContinueReading,
    Source { id: String },
    Category { name: String },
    Series { id: String },
    Tag { name: String },
    Title { text: String },
}

impl Default for FilterView {
    fn default() -> Self {
        FilterView::All
    }
}

impl From<FilterView> for ItemFilter {
    fn from(f: FilterView) -> Self {
        match f {
            FilterView::All => ItemFilter::All,
            FilterView::Starred => ItemFilter::Starred,
            FilterView::Saved => ItemFilter::Saved,
            FilterView::ContinueReading => ItemFilter::ContinueReading,
            FilterView::Source { id } => ItemFilter::Source(id),
            FilterView::Category { name } => ItemFilter::Category(name),
            FilterView::Series { id } => ItemFilter::Series(id),
            FilterView::Tag { name } => ItemFilter::Tag(name),
            FilterView::Title { text } => ItemFilter::Title(text),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QueryView {
    #[serde(default)]
    pub filter: FilterView,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub keep: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    200
}

impl From<QueryView> for ItemQuery {
    fn from(q: QueryView) -> Self {
        ItemQuery {
            filter: q.filter.into(),
            unread_only: q.unread_only,
            keep: q.keep,
            limit: q.limit,
            offset: q.offset,
        }
    }
}
