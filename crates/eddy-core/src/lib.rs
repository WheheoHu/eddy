//! The Eddy engine: owns the store, talks to the upstream (FreshRSS directly
//! or the sync server) and exposes the operations the native apps call.

mod content;
mod engine;
mod images;
mod platform;
mod upstream;

pub use content::LoadedDocument;
pub use engine::{Engine, EngineOptions, EngineStatus, Mode, RefreshSummary, SourceInfo};
pub use platform::{LogLevel, MemoryPlatform, Platform};

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
