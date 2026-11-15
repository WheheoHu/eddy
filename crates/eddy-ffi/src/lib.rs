//! UniFFI surface of the Eddy engine for Swift and Kotlin.

mod ai;
mod export;
mod media;
mod progress;
mod sync;
mod video;
mod views;

use std::path::PathBuf;
use std::sync::Arc;

use eddy_core::{Engine, EngineError, EngineOptions, LogLevel, Platform};

pub use ai::*;
pub use export::*;
pub use media::*;
pub use progress::*;
pub use sync::*;
pub use video::*;
pub use views::*;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EddyError {
    #[error("还没有连接 FreshRSS，请先在设置里填写。")]
    NotConfigured,
    #[error("找不到这篇内容。")]
    NotFound,
    /// The sync server cannot be reached. Local work continues.
    #[error("{0}")]
    Offline(String),
    /// The server no longer accepts this device: pair again.
    #[error("{0}")]
    Unauthorized(String),
    /// Client and server versions are too far apart to sync.
    #[error("{0}")]
    UpgradeRequired(String),
    #[error("{0}")]
    Failed(String),
}

impl From<EngineError> for EddyError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NotConfigured => EddyError::NotConfigured,
            EngineError::NotFound => EddyError::NotFound,
            EngineError::Offline(_) => EddyError::Offline(e.to_string()),
            EngineError::Unauthorized => EddyError::Unauthorized(e.to_string()),
            EngineError::UpgradeRequired(_) => EddyError::UpgradeRequired(e.to_string()),
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

/// Implemented by the host app: renders a page in a WebView (JavaScript,
/// the user's own sessions) and returns its HTML, or `None` on failure.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait WebPageRenderer: Send + Sync {
    async fn render(&self, url: String) -> Option<String>;
}

struct RendererAdapter(Arc<dyn WebPageRenderer>);

#[async_trait::async_trait]
impl eddy_core::WebRenderer for RendererAdapter {
    async fn render(&self, url: &str) -> Option<String> {
        self.0.render(url.to_string()).await
    }
}

/// Receives the running total of older articles added while FreshRSS
/// history loads.
#[uniffi::export(with_foreign)]
pub trait HistoryListener: Send + Sync {
    fn on_progress(&self, added: u32);
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ImportResultView {
    /// What to open: the PDF, or the book's first chapter.
    pub item_id: String,
    pub series_id: Option<String>,
    pub title: String,
    /// The file had been imported before.
    pub existing: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct RulePackView {
    pub name: String,
    pub sites: u32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct RulePackSummaryView {
    pub packs: Vec<RulePackView>,
    pub ftr_sites: u32,
}

impl From<eddy_core::RulePackSummary> for RulePackSummaryView {
    fn from(s: eddy_core::RulePackSummary) -> Self {
        RulePackSummaryView {
            packs: s
                .packs
                .into_iter()
                .map(|(name, sites)| RulePackView { name, sites })
                .collect(),
            ftr_sites: s.ftr_sites,
        }
    }
}

/// Chrome extension ID allowed to talk to the capture host.
#[uniffi::export]
pub fn chrome_extension_id() -> String {
    eddy_core::CHROME_EXTENSION_ID.to_string()
}

/// Registers the capture host with Chrome-family browsers; returns the
/// manifest files written.
#[uniffi::export]
pub fn install_native_host(host_path: String) -> Result<Vec<String>, EddyError> {
    let files = eddy_core::install_native_host(std::path::Path::new(&host_path))?;
    Ok(files
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
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

/// Where a renderer acts on mixed Chinese and Latin text: which ranges take
/// the Chinese font, and which characters get tighter or looser spacing.
#[uniffi::export]
pub fn typography_spans(text: String) -> Vec<TypoSpanView> {
    eddy_core::typography_spans(&text)
        .into_iter()
        .map(TypoSpanView::from)
        .collect()
}

/// Version of the Rust core, e.g. `0.2.0`. The app has its own version.
#[uniffi::export]
pub fn core_version() -> String {
    eddy_core::CORE_VERSION.to_string()
}

/// Short hash of the commit the core was built from.
#[uniffi::export]
pub fn core_commit() -> String {
    eddy_core::CORE_COMMIT.to_string()
}

/// Version of the sync protocol the core speaks.
#[uniffi::export]
pub fn core_protocol_version() -> u32 {
    eddy_core::SYNC_PROTOCOL_VERSION
}

#[derive(uniffi::Object)]
pub struct EddyEngine {
    pub(crate) engine: Engine,
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    #[uniffi::constructor]
    pub fn new(
        data_dir: String,
        platform: Arc<dyn PlatformServices>,
        default_cache_limit: u64,
    ) -> R<Arc<Self>> {
        let engine = Engine::open(
            &PathBuf::from(data_dir),
            Arc::new(PlatformAdapter(platform)),
            EngineOptions {
                default_cache_limit,
                server_role: false,
            },
        )?;
        engine.set_pdf_reader(Some(Arc::new(eddy_pdf::PdfiumReader::new())));
        Ok(Arc::new(EddyEngine { engine }))
    }

    pub fn device_id(&self) -> String {
        self.engine.device_id()
    }

    pub fn status(&self) -> R<StatusView> {
        Ok(self.engine.status()?.into())
    }

    pub async fn configure_freshrss(
        &self,
        url: String,
        username: String,
        password: String,
    ) -> R<AccountView> {
        let a = self
            .engine
            .configure_freshrss(&url, &username, &password)
            .await?;
        Ok(AccountView {
            base_url: a.base_url,
            username: a.username,
        })
    }

    pub fn remove_account(&self) -> R<()> {
        Ok(self.engine.remove_account()?)
    }

    pub async fn refresh(&self) -> R<RefreshSummaryView> {
        Ok(self.engine.refresh().await?.into())
    }

    /// Loads older FreshRSS articles page by page until the history is
    /// complete. Returns how many were added; returns at once when there is
    /// nothing left or a walk is already running.
    pub async fn backfill_history(&self, listener: Option<Arc<dyn HistoryListener>>) -> R<u32> {
        let callback = listener
            .map(|l| Arc::new(move |added| l.on_progress(added)) as eddy_core::HistoryCallback);
        Ok(self.engine.backfill_history(callback).await?)
    }

    pub fn sources(&self) -> R<Vec<SourceView>> {
        let rules = self.engine.full_text_rules()?;
        Ok(self
            .engine
            .sources()?
            .into_iter()
            .map(|s| SourceView::new(s, &rules))
            .collect())
    }

    pub fn items(&self, query: ItemQueryView) -> R<Vec<ItemView>> {
        Ok(self
            .engine
            .items(&query.into())?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// How many items `items` would list without its limit and offset.
    pub fn item_count(&self, query: ItemQueryView) -> R<u32> {
        Ok(self.engine.item_count(&query.into())?)
    }

    /// How many unread subscription articles in a list arrived by `until`
    /// (ms): what `mark_all_read` with the same arguments marks.
    pub fn unread_feed_count(&self, filter: ItemFilterView, until: i64) -> R<u32> {
        Ok(self.engine.unread_feed_count(&filter.into(), until)?)
    }

    /// Marks read the unread subscription articles in a list that arrived
    /// by `until` (ms); returns how many.
    pub async fn mark_all_read(&self, filter: ItemFilterView, until: i64) -> R<u32> {
        Ok(self.engine.mark_all_read(&filter.into(), until)?)
    }

    pub fn item(&self, id: String) -> R<Option<ItemView>> {
        Ok(self.engine.item(&id)?.map(Into::into))
    }

    pub async fn document(&self, item_id: String) -> R<Option<DocumentView>> {
        Ok(self.engine.document(&item_id).await?.map(Into::into))
    }

    /// Highlights a code block as `language` from now on (`None`: as the
    /// page declared or the guess). The next `document` call reflects it.
    pub fn set_code_language(
        &self,
        item_id: String,
        block_id: String,
        language: Option<String>,
    ) -> R<()> {
        Ok(self
            .engine
            .set_code_language(&item_id, &block_id, language.as_deref())?)
    }

    /// Language tokens there are grammars for, for a language picker.
    pub fn highlight_languages(&self) -> Vec<String> {
        eddy_core::highlight_languages()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// A translation as runs: the formulas it kept between dollars become
    /// math runs, to render like the original's.
    pub fn translated_runs(&self, text: String) -> Vec<RunView> {
        eddy_core::translated_runs(&text)
            .into_iter()
            .map(RunView::from)
            .collect()
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

    /// Full-text rules, top (highest priority) first.
    pub fn full_text_rules(&self) -> R<Vec<FullTextRuleView>> {
        Ok(self
            .engine
            .full_text_rules()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn set_full_text_rules(&self, rules: Vec<FullTextRuleView>) -> R<()> {
        Ok(self
            .engine
            .set_full_text_rules(rules.into_iter().map(Into::into).collect())?)
    }

    /// Puts a rule for this source alone at the top, replacing any it had;
    /// `None` removes it so the other rules apply.
    pub fn set_source_full_text_rule(
        &self,
        source_id: String,
        mode: Option<FullTextModeView>,
    ) -> R<()> {
        Ok(self
            .engine
            .set_source_full_text_rule(&source_id, mode.map(Into::into))?)
    }

    pub async fn fetch_full_text(&self, item_id: String) -> R<DocumentView> {
        Ok(self.engine.fetch_full_text(&item_id).await?.into())
    }

    /// Shows a subscription article's full text or the feed's content; the
    /// page is fetched only the first time. The choice is remembered.
    pub async fn set_full_text(&self, item_id: String, on: bool) -> R<DocumentView> {
        Ok(self.engine.set_full_text(&item_id, on).await?.into())
    }

    /// Applies the full-text and automatic AI rules to an item being
    /// opened: the full text they fetched, and what they ask to show.
    pub async fn apply_rules(&self, item_id: String) -> R<RulesOnOpenView> {
        Ok(self.engine.apply_rules(&item_id).await?.into())
    }

    /// Whether opening the item is about to fetch its full text.
    pub fn full_text_on_open(&self, item_id: String) -> R<bool> {
        Ok(self.engine.full_text_on_open(&item_id)?)
    }

    pub async fn save_url(&self, url: String) -> R<ItemView> {
        Ok(self.engine.save_url(&url).await?.into())
    }

    pub fn spool_dir(&self) -> String {
        self.engine.spool_dir().to_string_lossy().into_owned()
    }

    pub async fn import_captures(&self) -> R<u32> {
        Ok(self.engine.import_captures().await?)
    }

    /// The saved page at `url`, for showing it from the browser.
    pub fn saved_item_for_url(&self, url: String) -> R<Option<String>> {
        Ok(self.engine.saved_item_for_url(&url)?)
    }

    pub fn import_rule_pack(&self, name: String, contents: String) -> R<RulePackSummaryView> {
        Ok(self.engine.import_rule_pack(&name, &contents)?.into())
    }

    pub fn import_ftr_rule(&self, domain: String, contents: String) -> R<RulePackSummaryView> {
        Ok(self.engine.import_ftr_rule(&domain, &contents)?.into())
    }

    pub fn remove_rule_pack(&self, name: String) -> R<RulePackSummaryView> {
        Ok(self.engine.remove_rule_pack(&name)?.into())
    }

    pub fn rule_packs(&self) -> RulePackSummaryView {
        self.engine.rule_packs().into()
    }

    pub fn set_web_renderer(&self, renderer: Option<Arc<dyn WebPageRenderer>>) -> R<()> {
        let renderer =
            renderer.map(|r| Arc::new(RendererAdapter(r)) as Arc<dyn eddy_core::WebRenderer>);
        Ok(self.engine.set_web_renderer(renderer)?)
    }

    /// Imports an EPUB (as a publication) or a PDF (as a saved document).
    pub async fn import_file(&self, path: String) -> R<ImportResultView> {
        let imported = self.engine.import_file(std::path::Path::new(&path)).await?;
        Ok(ImportResultView {
            item_id: imported.item_id,
            series_id: imported.series_id,
            title: imported.title,
            existing: imported.existing,
        })
    }

    /// Local path of an item's original file (an imported PDF), downloaded
    /// from the sync server when needed.
    pub async fn content_file(&self, item_id: String) -> R<Option<String>> {
        Ok(self
            .engine
            .content_file(&item_id)
            .await?
            .map(|p| p.to_string_lossy().into_owned()))
    }

    /// The chapter an in-book link (`epub://…`) points to.
    pub fn item_for_link(&self, url: String) -> R<Option<String>> {
        Ok(self.engine.item_for_link(&url)?)
    }
}
