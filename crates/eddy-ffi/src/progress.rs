use eddy_core as core;

use crate::{EddyEngine, EddyError};

#[derive(uniffi::Record, Clone, Debug)]
pub struct ResumePointView {
    pub block_id: Option<String>,
    /// UTF-16 offset within the block's layout text.
    pub offset: u32,
    pub progression: f64,
    pub updated_at: i64,
    pub device: Option<String>,
    pub from_other_device: bool,
}

impl From<core::ResumePoint> for ResumePointView {
    fn from(r: core::ResumePoint) -> Self {
        ResumePointView {
            block_id: r.block_id,
            offset: r.offset,
            progression: r.locator.progression,
            updated_at: r.updated_at,
            device: r.device,
            from_other_device: r.from_other_device,
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
            chapters: p.chapters.into_iter().map(|c| ChapterLinkView { title: c.title, url: c.url }).collect(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SeriesView {
    pub id: String,
    pub title: String,
    pub toc_url: Option<String>,
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
            chapters: s.chapters,
            finished: s.finished,
            progress: s.progress,
            next_item_id: s.next_item_id,
        }
    }
}

type R<T> = Result<T, EddyError>;

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// Reports the top visible block and UTF-16 offset; returns the overall
    /// progression. Throttled unless `force`.
    pub fn record_progress(&self, item_id: String, block_id: String, offset: u32, force: bool) -> R<Option<f64>> {
        Ok(self.engine.record_progress(&item_id, &block_id, offset, force)?.map(|l| l.progression))
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

    pub fn create_series(&self, title: String, toc_url: Option<String>, chapters: Vec<ChapterLinkView>) -> R<String> {
        let chapters: Vec<(String, String)> = chapters.into_iter().map(|c| (c.title, c.url)).collect();
        Ok(self.engine.create_series(&title, toc_url.as_deref(), &chapters)?)
    }

    pub fn add_to_series(&self, series_id: String, item_id: String, chapter_index: u32) -> R<()> {
        Ok(self.engine.add_to_series(&series_id, &item_id, chapter_index)?)
    }

    pub fn remove_from_series(&self, item_id: String) -> R<()> {
        Ok(self.engine.remove_from_series(&item_id)?)
    }

    pub fn rename_series(&self, series_id: String, title: String) -> R<()> {
        Ok(self.engine.rename_series(&series_id, &title)?)
    }

    pub fn delete_series(&self, series_id: String) -> R<()> {
        Ok(self.engine.delete_series(&series_id)?)
    }

    pub fn series_list(&self) -> R<Vec<SeriesView>> {
        Ok(self.engine.series_list()?.into_iter().map(Into::into).collect())
    }

    pub fn series_info(&self, series_id: String) -> R<Option<SeriesView>> {
        Ok(self.engine.series_info(&series_id)?.map(Into::into))
    }
}
