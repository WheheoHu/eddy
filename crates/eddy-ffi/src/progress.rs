use eddy_core as core;

use crate::{EddyEngine, EddyError};

#[derive(uniffi::Record, Clone, Debug)]
pub struct ResumePointView {
    pub block_id: Option<String>,
    /// UTF-16 offset within the block's layout text.
    pub offset: u32,
    /// Page of a PDF (0-based).
    pub page: Option<u32>,
    pub progression: f64,
    pub updated_at: i64,
    pub device: Option<String>,
    pub from_other_device: bool,
    /// This device's own last position, for "back to where I was here".
    pub local_block_id: Option<String>,
    pub local_offset: u32,
}

impl From<core::ResumePoint> for ResumePointView {
    fn from(r: core::ResumePoint) -> Self {
        ResumePointView {
            block_id: r.block_id,
            offset: r.offset,
            page: r.page,
            progression: r.locator.progression,
            updated_at: r.updated_at,
            device: r.device,
            from_other_device: r.from_other_device,
            local_block_id: r.local_block_id,
            local_offset: r.local_offset,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ChapterLinkView {
    pub title: String,
    pub url: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SeriesProposalView {
    pub title: String,
    pub toc_url: String,
    pub chapters: Vec<ChapterLinkView>,
}

impl From<core::SeriesProposal> for SeriesProposalView {
    fn from(p: core::SeriesProposal) -> Self {
        SeriesProposalView {
            title: p.title,
            toc_url: p.toc_url,
            chapters: p
                .chapters
                .into_iter()
                .map(|c| ChapterLinkView {
                    title: c.title,
                    url: c.url,
                })
                .collect(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SeriesView {
    pub id: String,
    pub title: String,
    pub toc_url: Option<String>,
    pub author: Option<String>,
    /// Cover image URL for `image`.
    pub cover: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    pub progress: f64,
    pub next_item_id: Option<String>,
}

impl From<core::SeriesInfo> for SeriesView {
    fn from(s: core::SeriesInfo) -> Self {
        SeriesView {
            id: s.id,
            title: s.title,
            toc_url: s.toc_url,
            author: s.author,
            cover: s.cover,
            chapters: s.chapters,
            finished: s.finished,
            progress: s.progress,
            next_item_id: s.next_item_id,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CollectionKindView {
    Book,
    Pdf,
    WebPage,
    WebSeries,
}

impl From<core::CollectionKind> for CollectionKindView {
    fn from(k: core::CollectionKind) -> Self {
        match k {
            core::CollectionKind::Book => CollectionKindView::Book,
            core::CollectionKind::Pdf => CollectionKindView::Pdf,
            core::CollectionKind::WebPage => CollectionKindView::WebPage,
            core::CollectionKind::WebSeries => CollectionKindView::WebSeries,
        }
    }
}

/// Anything that is not a subscription article: a book, a PDF, a saved
/// page or a web series.
#[derive(uniffi::Record, Clone, Debug)]
pub struct CollectionEntryView {
    /// The series ID, or the item ID of a page or PDF saved on its own.
    pub id: String,
    pub kind: CollectionKindView,
    pub series_id: Option<String>,
    pub title: String,
    pub author: Option<String>,
    /// A book's own cover image URL for `image`.
    pub cover: Option<String>,
    /// The page, or a series' table of contents.
    pub url: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    pub progress: f64,
    /// The chapter to open: where reading stopped.
    pub resume_item_id: Option<String>,
    pub added_at: i64,
    pub updated_at: i64,
    pub last_read_at: Option<i64>,
    pub text_len: u32,
    pub lang: Option<String>,
}

impl From<core::CollectionEntry> for CollectionEntryView {
    fn from(e: core::CollectionEntry) -> Self {
        CollectionEntryView {
            id: e.id,
            kind: e.kind.into(),
            series_id: e.series_id,
            title: e.title,
            author: e.author,
            cover: e.cover,
            url: e.url,
            chapters: e.chapters,
            finished: e.finished,
            progress: e.progress,
            resume_item_id: e.resume_item_id,
            added_at: e.added_at,
            updated_at: e.updated_at,
            last_read_at: e.last_read_at,
            text_len: e.text_len,
            lang: e.lang,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TextRangeView {
    pub start: u32,
    pub end: u32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SearchResultView {
    pub item: crate::ItemView,
    pub snippet: String,
    /// Highlighted ranges in `snippet`, UTF-16 offsets.
    pub highlights: Vec<TextRangeView>,
}

type R<T> = Result<T, EddyError>;

#[uniffi::export]
impl EddyEngine {
    pub fn search(&self, query: String, limit: u32) -> R<Vec<SearchResultView>> {
        Ok(self
            .engine
            .search(&query, limit)?
            .into_iter()
            .map(|r| SearchResultView {
                item: r.item.into(),
                snippet: r.snippet,
                highlights: r
                    .highlights
                    .into_iter()
                    .map(|(start, end)| TextRangeView { start, end })
                    .collect(),
            })
            .collect())
    }

    pub fn rebuild_search_index(&self) -> R<u32> {
        Ok(self.engine.rebuild_search_index()?)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// Reports the top visible block and UTF-16 offset; returns the overall
    /// progression. Throttled unless `force`.
    pub fn record_progress(
        &self,
        item_id: String,
        block_id: String,
        offset: u32,
        force: bool,
    ) -> R<Option<f64>> {
        Ok(self
            .engine
            .record_progress(&item_id, &block_id, offset, force)?
            .map(|l| l.progression))
    }

    /// Reports the page a PDF view shows (0-based) out of `page_count`;
    /// returns the overall progression. Throttled unless `force`.
    pub fn record_page_progress(
        &self,
        item_id: String,
        page: u32,
        page_count: u32,
        force: bool,
    ) -> R<Option<f64>> {
        Ok(self
            .engine
            .record_page_progress(&item_id, page, page_count, force)?
            .map(|l| l.progression))
    }

    pub fn set_finished(&self, item_id: String, finished: bool) -> R<()> {
        Ok(self.engine.set_finished(&item_id, finished)?)
    }

    pub fn resume_point(&self, item_id: String) -> R<Option<ResumePointView>> {
        Ok(self.engine.resume_point(&item_id)?.map(Into::into))
    }

    pub fn detect_series_in(&self, item_id: String) -> R<Option<SeriesProposalView>> {
        Ok(self.engine.detect_series_in(&item_id)?.map(Into::into))
    }

    pub async fn detect_series_at(&self, url: String) -> R<Option<SeriesProposalView>> {
        Ok(self.engine.detect_series_at(&url).await?.map(Into::into))
    }

    pub fn create_series(
        &self,
        title: String,
        toc_url: Option<String>,
        chapters: Vec<ChapterLinkView>,
    ) -> R<String> {
        let chapters: Vec<(String, String)> =
            chapters.into_iter().map(|c| (c.title, c.url)).collect();
        Ok(self
            .engine
            .create_series(&title, toc_url.as_deref(), &chapters)?)
    }

    pub fn add_to_series(&self, series_id: String, item_id: String, chapter_index: u32) -> R<()> {
        Ok(self
            .engine
            .add_to_series(&series_id, &item_id, chapter_index)?)
    }

    pub fn remove_from_series(&self, item_id: String) -> R<()> {
        Ok(self.engine.remove_from_series(&item_id)?)
    }

    pub fn rename_series(&self, series_id: String, title: String) -> R<()> {
        Ok(self.engine.rename_series(&series_id, &title)?)
    }

    /// Deletes a series together with its chapters.
    pub fn delete_series(&self, series_id: String) -> R<()> {
        Ok(self.engine.delete_series(&series_id)?)
    }

    pub fn collection(&self) -> R<Vec<CollectionEntryView>> {
        Ok(self
            .engine
            .collection()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Deletes a collection entry on every device: a series with its
    /// chapters, or a saved page or PDF. Subscription articles cannot be
    /// deleted.
    pub async fn delete_entry(&self, id: String) -> R<()> {
        Ok(self.engine.delete_entry(&id).await?)
    }

    pub fn series_list(&self) -> R<Vec<SeriesView>> {
        Ok(self
            .engine
            .series_list()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn series_info(&self, series_id: String) -> R<Option<SeriesView>> {
        Ok(self.engine.series_info(&series_id)?.map(Into::into))
    }
}
