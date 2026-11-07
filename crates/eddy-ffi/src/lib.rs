//! UniFFI surface of the Eddy engine for Swift and Kotlin.

mod views;

use std::path::PathBuf;
use std::sync::Arc;

use eddy_core::{Engine, EngineError, EngineOptions, LogLevel, Platform};

pub use views::*;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EddyError {
    #[error("还没有连接 FreshRSS，请先在设置里填写。")]
    NotConfigured,
    #[error("找不到这篇内容。")]
    NotFound,
    #[error("{0}")]
    Failed(String),
}

impl From<EngineError> for EddyError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NotConfigured => EddyError::NotConfigured,
            EngineError::NotFound => EddyError::NotFound,
            other => EddyError::Failed(other.to_string()),
        }
    }
}

type R<T> = Result<T, EddyError>;

/// Implemented by the host app: keychain access and logging.
#[uniffi::export(with_foreign)]
pub trait PlatformServices: Send + Sync {
    fn secret_get(&self, key: String) -> Option<String>;
    fn secret_set(&self, key: String, value: String) -> bool;
    fn secret_delete(&self, key: String);
    fn log(&self, level: String, message: String);
}

struct PlatformAdapter(Arc<dyn PlatformServices>);

impl Platform for PlatformAdapter {
    fn secret_get(&self, key: &str) -> Option<String> {
        self.0.secret_get(key.to_string())
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        self.0.secret_set(key.to_string(), value.to_string())
    }

    fn secret_delete(&self, key: &str) {
        self.0.secret_delete(key.to_string())
    }

    fn log(&self, level: LogLevel, message: &str) {
        let level = match level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        self.0.log(level.to_string(), message.to_string())
    }
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(uniffi::Object)]
pub struct EddyEngine {
    engine: Engine,
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    #[uniffi::constructor]
    pub fn new(data_dir: String, platform: Arc<dyn PlatformServices>, default_cache_limit: u64) -> R<Arc<Self>> {
        let engine = Engine::open(
            &PathBuf::from(data_dir),
            Arc::new(PlatformAdapter(platform)),
            EngineOptions { default_cache_limit },
        )?;
        Ok(Arc::new(EddyEngine { engine }))
    }

    pub fn device_id(&self) -> String {
        self.engine.device_id()
    }

    pub fn status(&self) -> R<StatusView> {
        Ok(self.engine.status()?.into())
    }

    pub async fn configure_freshrss(&self, url: String, username: String, password: String) -> R<AccountView> {
        let a = self.engine.configure_freshrss(&url, &username, &password).await?;
        Ok(AccountView { base_url: a.base_url, username: a.username })
    }

    pub fn remove_account(&self) -> R<()> {
        Ok(self.engine.remove_account()?)
    }

    pub async fn refresh(&self) -> R<RefreshSummaryView> {
        Ok(self.engine.refresh().await?.into())
    }

    pub fn sources(&self) -> R<Vec<SourceView>> {
        Ok(self.engine.sources()?.into_iter().map(Into::into).collect())
    }

    pub fn items(&self, query: ItemQueryView) -> R<Vec<ItemView>> {
        Ok(self.engine.items(&query.into())?.into_iter().map(Into::into).collect())
    }

    pub fn item(&self, id: String) -> R<Option<ItemView>> {
        Ok(self.engine.item(&id)?.map(Into::into))
    }

    pub async fn document(&self, item_id: String) -> R<Option<DocumentView>> {
        Ok(self.engine.document(&item_id).await?.map(Into::into))
    }

    pub async fn set_read(&self, item_id: String, read: bool) -> R<()> {
        Ok(self.engine.set_read(&item_id, read).await?)
    }

    pub async fn set_starred(&self, item_id: String, starred: bool) -> R<()> {
        Ok(self.engine.set_starred(&item_id, starred).await?)
    }

    pub async fn image(&self, url: String) -> R<ImageData> {
        let (bytes, mime) = self.engine.image(&url).await?;
        Ok(ImageData { bytes, mime })
    }

    pub fn cache_limit(&self) -> R<u64> {
        Ok(self.engine.cache_limit()?)
    }

    pub fn set_cache_limit(&self, bytes: u64) -> R<()> {
        Ok(self.engine.set_cache_limit(bytes)?)
    }

    pub fn cache_usage(&self) -> R<u64> {
        Ok(self.engine.cache_usage()?)
    }

    pub fn set_source_full_text(&self, source_id: String, mode: FullTextModeView) -> R<()> {
        Ok(self.engine.set_source_full_text(&source_id, mode.into())?)
    }
}
