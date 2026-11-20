//! The Eddy engine: owns the store, talks to the upstream (FreshRSS directly
//! or the sync server) and exposes the operations the native apps call.

mod ai;
mod collection;
mod content;
mod engine;
mod epub_export;
mod erase;
mod export;
mod favicon;
mod fulltext;
mod hub;
mod images;
mod import;
mod media;
mod native_host;
mod pdf_layout;
mod pdf_listing;
mod pdf_table;
mod platform;
mod progress;
mod rules;
mod search;
mod server;
mod subscribe;
mod upstream;
mod video;

pub use ai::{
    AiRole, AiTestResult, BatchCallback, ItemTagsResult, SummaryResult, TagInfo, TranslationResult,
};
pub use collection::{CollectionEntry, CollectionKind};
pub use content::LoadedDocument;
pub use eddy_ai::{
    AiProtocol, AiSettings, AutoAction, AutoRule, MAX_CONCURRENCY, ProviderSettings, RuleScope,
    Summary,
};
pub use eddy_doc::{
    Segment, TypoKind, TypoSpan, highlight_languages, translated_runs, typography_spans,
};
pub use eddy_extract::{ChapterLink, SeriesProposal};
pub use eddy_fetch::WebRenderer;
pub use eddy_media::{MediaItem, MediaSet, MediaStatus, MediaType, is_media_url};
pub use eddy_video::{
    PipelinePhase, ProtocolKind, StageEndpointSettings, ToolchainStatus, TranscriptPolicy,
    VideoPipelineMode, VideoProbe, VideoSettings, VideoStageRole, is_video_file_path, is_video_url,
};
pub use engine::{
    Engine, EngineOptions, EngineStatus, Mode, RefreshSummary, SourceInfo, SourceStats, SyncPhase,
    SyncProgress,
};
pub use erase::SignOut;
pub use export::{
    ExportContentMode, ExportFormat, ExportOptions, ExportProgress, ExportProgressCallback,
    ExportRequest, ExportScope, PdfExportOutcome, PreparedExportChapter, PreparedPdfExport,
    sanitize_filename,
};
pub use fulltext::RulesOnOpen;
pub use hub::{ServerStatus, plausible_hlc};
pub use import::{
    Imported, MAX_IMPORT_BYTES, PdfBox, PdfImage, PdfLine, PdfPage, PdfReader, PdfRule, PdfRun,
    PdfText,
};
pub use media::{GalleryEntry, GalleryScope};
pub use native_host::{CHROME_EXTENSION_ID, NATIVE_HOST_NAME, install_native_host};
pub use platform::{LogLevel, MemoryPlatform, Platform};
pub use progress::{ResumePoint, SeriesInfo};
pub use rules::RulePackSummary;
pub use search::SearchResult;
pub use server::{AppliedOps, OpsPage, ServerOverview};
pub use subscribe::AddedFeed;
pub use upstream::HistoryCallback;
pub use video::{
    StartVideoJobOptions, VideoJobCallback, VideoJobInfo, VideoJobTarget, record_to_remote,
};

pub use eddy_store::{ItemFilter, ItemQuery, ItemRow};

/// Version of the core (the Rust crates: engine, server, CLI). The apps
/// carry their own version; see docs/adr/0009.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the sync protocol this core speaks.
pub const SYNC_PROTOCOL_VERSION: u32 = eddy_sync::PROTOCOL_VERSION;

/// Short hash of the commit this core was built from, or `unknown`.
pub const CORE_COMMIT: &str = env!("EDDY_GIT_HASH");

/// `0.2.0 (1a2b3c4)`, for About screens and `--version`.
pub fn core_version() -> String {
    format!("{CORE_VERSION} ({CORE_COMMIT})")
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("本地数据出错：{0}")]
    Store(#[from] eddy_store::StoreError),
    #[error("{0}")]
    Upstream(#[from] eddy_source_greader::GReaderError),
    #[error("还没有连接 FreshRSS，请先在设置里填写。")]
    NotConfigured,
    #[error("找不到这篇内容。")]
    NotFound,
    #[error("网络请求失败：{0}")]
    Network(String),
    /// The sync server cannot be reached; local work continues and is
    /// uploaded when it comes back.
    #[error("连不上服务端：{0}")]
    Offline(String),
    #[error("服务端拒绝了这台设备，请重新配对。")]
    Unauthorized,
    /// Client and server speak incompatible protocol versions.
    #[error("{0}")]
    UpgradeRequired(String),
    #[error("{0}")]
    Invalid(String),
}

impl From<serde_json::Error> for EngineError {
    fn from(e: serde_json::Error) -> Self {
        EngineError::Store(eddy_store::StoreError::Json(e))
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[cfg(test)]
mod tests;
