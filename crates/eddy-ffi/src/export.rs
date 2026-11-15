use std::collections::HashMap;
use std::sync::Arc;

use eddy_core as core;

use crate::{BlockView, EddyEngine, EddyError, SummaryView};

type R<T> = Result<T, EddyError>;

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormatView {
    Pdf,
    Epub,
}

impl From<ExportFormatView> for core::ExportFormat {
    fn from(v: ExportFormatView) -> Self {
        match v {
            ExportFormatView::Pdf => core::ExportFormat::Pdf,
            ExportFormatView::Epub => core::ExportFormat::Epub,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportContentModeView {
    Original,
    Bilingual,
    Translation,
}

impl From<ExportContentModeView> for core::ExportContentMode {
    fn from(v: ExportContentModeView) -> Self {
        match v {
            ExportContentModeView::Original => core::ExportContentMode::Original,
            ExportContentModeView::Bilingual => core::ExportContentMode::Bilingual,
            ExportContentModeView::Translation => core::ExportContentMode::Translation,
        }
    }
}

impl From<core::ExportContentMode> for ExportContentModeView {
    fn from(v: core::ExportContentMode) -> Self {
        match v {
            core::ExportContentMode::Original => ExportContentModeView::Original,
            core::ExportContentMode::Bilingual => ExportContentModeView::Bilingual,
            core::ExportContentMode::Translation => ExportContentModeView::Translation,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportScopeView {
    Whole,
    CurrentChapter,
}

impl From<ExportScopeView> for core::ExportScope {
    fn from(v: ExportScopeView) -> Self {
        match v {
            ExportScopeView::Whole => core::ExportScope::Whole,
            ExportScopeView::CurrentChapter => core::ExportScope::CurrentChapter,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ExportOptionsView {
    pub exportable: bool,
    pub series_id: Option<String>,
    pub chapter_item_id: Option<String>,
    pub series_title: Option<String>,
    pub item_title: String,
    pub whole_filename: String,
    pub chapter_filename: Option<String>,
    pub can_choose_scope: bool,
    pub is_book: bool,
    pub chapter_count: u32,
    pub has_cached_summary_whole: bool,
    pub has_cached_summary_chapter: bool,
    pub can_passthrough_epub: bool,
    pub can_passthrough_pdf: bool,
}

impl From<core::ExportOptions> for ExportOptionsView {
    fn from(o: core::ExportOptions) -> Self {
        ExportOptionsView {
            exportable: o.exportable,
            series_id: o.series_id,
            chapter_item_id: o.chapter_item_id,
            series_title: o.series_title,
            item_title: o.item_title,
            whole_filename: o.whole_filename,
            chapter_filename: o.chapter_filename,
            can_choose_scope: o.can_choose_scope,
            is_book: o.is_book,
            chapter_count: o.chapter_count,
            has_cached_summary_whole: o.has_cached_summary_whole,
            has_cached_summary_chapter: o.has_cached_summary_chapter,
            can_passthrough_epub: o.can_passthrough_epub,
            can_passthrough_pdf: o.can_passthrough_pdf,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ExportRequestView {
    pub job_id: String,
    pub target_id: String,
    pub destination_path: String,
    pub scope: ExportScopeView,
    pub mode: ExportContentModeView,
    pub include_summary: bool,
}

impl From<ExportRequestView> for core::ExportRequest {
    fn from(r: ExportRequestView) -> Self {
        core::ExportRequest {
            job_id: r.job_id,
            target_id: r.target_id,
            destination_path: r.destination_path,
            scope: r.scope.into(),
            mode: r.mode.into(),
            include_summary: r.include_summary,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ExportProgressView {
    pub phase: String,
    pub completed: u32,
    pub total: u32,
}

#[uniffi::export(with_foreign)]
pub trait ExportProgressListener: Send + Sync {
    fn on_progress(&self, progress: ExportProgressView);
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PreparedExportChapterView {
    pub item_id: String,
    pub title: String,
    pub author: Option<String>,
    pub source_title: Option<String>,
    pub url: Option<String>,
    pub published_at: i64,
    pub chapter_level: u32,
    pub blocks: Vec<BlockView>,
    pub translations: HashMap<String, String>,
    pub summary: Option<SummaryView>,
}

impl From<core::PreparedExportChapter> for PreparedExportChapterView {
    fn from(c: core::PreparedExportChapter) -> Self {
        PreparedExportChapterView {
            item_id: c.item_id,
            title: c.title,
            author: c.author,
            source_title: c.source_title,
            url: c.url,
            published_at: c.published_at,
            chapter_level: c.chapter_level,
            blocks: c.document.blocks.into_iter().map(Into::into).collect(),
            translations: c.translations,
            summary: c.summary.map(|s| SummaryView {
                tldr: s.tldr,
                points: s.points,
                cached: true,
            }),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PreparedPdfExportView {
    pub title: String,
    pub author: Option<String>,
    pub mode: ExportContentModeView,
    pub chapters: Vec<PreparedExportChapterView>,
}

impl From<core::PreparedPdfExport> for PreparedPdfExportView {
    fn from(p: core::PreparedPdfExport) -> Self {
        PreparedPdfExportView {
            title: p.title,
            author: p.author,
            mode: p.mode.into(),
            chapters: p.chapters.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum PdfExportOutcomeView {
    CopiedOriginal,
    Prepared { payload: PreparedPdfExportView },
}

impl From<core::PdfExportOutcome> for PdfExportOutcomeView {
    fn from(o: core::PdfExportOutcome) -> Self {
        match o {
            core::PdfExportOutcome::CopiedOriginal => PdfExportOutcomeView::CopiedOriginal,
            core::PdfExportOutcome::Prepared(p) => {
                PdfExportOutcomeView::Prepared { payload: p.into() }
            }
        }
    }
}

fn wrap_listener(
    listener: Option<Arc<dyn ExportProgressListener>>,
) -> Option<core::ExportProgressCallback> {
    listener.map(|l| {
        Arc::new(move |p: core::ExportProgress| {
            l.on_progress(ExportProgressView {
                phase: p.phase,
                completed: p.completed,
                total: p.total,
            });
        }) as core::ExportProgressCallback
    })
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    pub fn export_options(&self, target_id: String, from_reader: bool) -> R<ExportOptionsView> {
        Ok(self.engine.export_options(&target_id, from_reader)?.into())
    }

    pub fn cancel_export(&self, job_id: String) {
        self.engine.cancel_export(&job_id)
    }

    pub async fn export_epub(
        &self,
        request: ExportRequestView,
        listener: Option<Arc<dyn ExportProgressListener>>,
    ) -> R<()> {
        Ok(self
            .engine
            .export_epub(request.into(), wrap_listener(listener))
            .await?)
    }

    pub async fn export_or_prepare_pdf(
        &self,
        request: ExportRequestView,
        listener: Option<Arc<dyn ExportProgressListener>>,
    ) -> R<PdfExportOutcomeView> {
        Ok(self
            .engine
            .export_or_prepare_pdf(request.into(), wrap_listener(listener))
            .await?
            .into())
    }
}
