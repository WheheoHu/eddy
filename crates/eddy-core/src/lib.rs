//! The Eddy engine: owns the store, talks to the upstream (FreshRSS directly
//! or the sync server) and exposes the operations the native apps call.

mod ai;
mod content;
mod engine;
mod fulltext;
mod hub;
mod images;
mod native_host;
mod platform;
mod progress;
mod rules;
mod search;
mod server;
mod upstream;

pub use ai::{BatchCallback, ItemTagsResult, SummaryResult, TagInfo, TranslationResult};
pub use content::LoadedDocument;
pub use eddy_ai::{AiSettings, AutoAction, AutoRule, ProviderSettings, RuleScope, Summary};
pub use eddy_extract::{ChapterLink, SeriesProposal};
pub use eddy_fetch::WebRenderer;
pub use engine::{Engine, EngineOptions, EngineStatus, Mode, RefreshSummary, SourceInfo};
pub use hub::{ServerStatus, plausible_hlc};
pub use native_host::{CHROME_EXTENSION_ID, NATIVE_HOST_NAME, install_native_host};
pub use platform::{LogLevel, MemoryPlatform, Platform};
pub use progress::{ResumePoint, SeriesInfo};
pub use rules::RulePackSummary;
pub use search::SearchResult;

pub use eddy_store::{ItemFilter, ItemQuery, ItemRow};

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
