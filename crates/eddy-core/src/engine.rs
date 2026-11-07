use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eddy_model::{
    Account, Entity, FullTextMode, HlcClock, Op, Source, SourceKind, fields, ids, now_ms,
};
use eddy_fetch::{Fetcher, WebRenderer};
use eddy_source_greader::{Credentials, GReaderClient, normalize_base};
use eddy_store::{ItemQuery, ItemRow, Store};
use parking_lot::RwLock;

use crate::content::{Ingested, LoadedDocument};
use crate::platform::{LogLevel, Platform, account_secret_key};
use crate::upstream::{DirectUpstream, Upstream};
use crate::{EngineError, Result};

#[derive(Clone, Debug)]
pub struct EngineOptions {
    /// Cache limit used until the user sets one (bytes).
    pub default_cache_limit: u64,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions { default_cache_limit: 2 * 1024 * 1024 * 1024 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No upstream configured yet: saved pages still work.
    Unconfigured,
    /// Talking to FreshRSS directly.
    Standalone,
    /// Syncing through eddy-server.
    Server,
}

#[derive(Clone, Debug)]
pub struct EngineStatus {
    pub mode: Mode,
    pub account: Option<Account>,
    pub last_refresh: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshSummary {
    pub new_items: u32,
    pub state_changes: u32,
    pub sources: u32,
    /// IDs of the newly ingested items.
    pub new_item_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub source: Source,
    pub unread: u32,
}

pub(crate) struct Inner {
    pub store: Store,
    pub platform: Arc<dyn Platform>,
    pub clock: HlcClock,
    pub http: reqwest::Client,
    pub upstream: RwLock<Option<Arc<dyn Upstream>>>,
    pub direct: RwLock<Option<Arc<DirectUpstream>>>,
    pub refresh_lock: tokio::sync::Mutex<()>,
    pub data_dir: PathBuf,
    pub options: EngineOptions,
    pub fetcher: RwLock<Arc<Fetcher>>,
    pub web_renderer: RwLock<Option<Arc<dyn WebRenderer>>>,
}

/// Cheap to clone; all clones share one engine.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl Engine {
    pub fn open(data_dir: &Path, platform: Arc<dyn Platform>, options: EngineOptions) -> Result<Engine> {
        let store = Store::open(data_dir)?;
        let device_id = store.device_id()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Eddy/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let inner = Arc::new(Inner {
            store,
            platform,
            clock: HlcClock::new(device_id),
            http,
            upstream: RwLock::new(None),
            direct: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
            data_dir: data_dir.to_path_buf(),
            options,
            fetcher: RwLock::new(Arc::new(
                Fetcher::new(Default::default(), None).map_err(|e| EngineError::Network(e.to_string()))?,
            )),
            web_renderer: RwLock::new(None),
        });
        inner.rebuild_fetcher()?;
        inner.store.upsert_source(&Source {
            id: ids::saved_source_id(),
            account_id: None,
            upstream_id: None,
            kind: SourceKind::Saved,
            title: "保存的网页".into(),
            category: None,
            site_url: None,
            full_text: FullTextMode::Never,
            truncated: false,
        })?;
        inner.restore_upstream()?;
        Ok(Engine { inner })
    }

    pub fn device_id(&self) -> String {
        self.inner.clock.node().to_string()
    }

    pub fn status(&self) -> Result<EngineStatus> {
        let inner = &self.inner;
        let account = inner.direct.read().as_ref().map(|d| d.account.clone());
        let mode = match inner.upstream.read().as_ref().map(|u| u.kind()) {
            Some("direct") => Mode::Standalone,
            Some(_) => Mode::Server,
            None => Mode::Unconfigured,
        };
        Ok(EngineStatus {
            mode,
            account,
            last_refresh: inner.store.meta_get("last_refresh")?.and_then(|v| v.parse().ok()),
            last_error: inner.store.meta_get("last_error")?,
        })
    }

    /// Verifies the FreshRSS credentials, stores the password in the platform
    /// keychain and switches to standalone mode.
    pub async fn configure_freshrss(&self, url: &str, username: &str, password: &str) -> Result<Account> {
        let base = normalize_base(url)?;
        let client = GReaderClient::new(Credentials { base_url: base.clone(), username: username.into(), password: password.into() })?;
        client.login().await?;
        client.subscriptions().await?;
        let account = Account {
            id: ids::account_id("greader", &base, username.trim()),
            kind: "greader".into(),
            base_url: base,
            username: username.trim().to_string(),
            created_at: now_ms(),
        };
        if !self.inner.platform.secret_set(&account_secret_key(&account.id), password) {
            return Err(EngineError::Invalid("无法把密码保存到钥匙串。".into()));
        }
        for old in self.inner.store.accounts()? {
            if old.id != account.id {
                self.inner.platform.secret_delete(&account_secret_key(&old.id));
                self.inner.store.delete_account(&old.id)?;
            }
        }
        self.inner.store.upsert_account(&account)?;
        self.inner.set_direct(DirectUpstream { account: account.clone(), client });
        Ok(account)
    }

    pub fn remove_account(&self) -> Result<()> {
        for account in self.inner.store.accounts()? {
            self.inner.platform.secret_delete(&account_secret_key(&account.id));
            self.inner.store.delete_account(&account.id)?;
        }
        *self.inner.direct.write() = None;
        *self.inner.upstream.write() = None;
        Ok(())
    }

    /// Syncs with the active upstream and trims the cache.
    pub async fn refresh(&self) -> Result<RefreshSummary> {
        let inner = &self.inner;
        let _guard = inner.refresh_lock.lock().await;
        let upstream = inner.upstream.read().clone().ok_or(EngineError::NotConfigured)?;
        let result = upstream.sync(inner).await;
        match &result {
            Ok(summary) => {
                inner.store.meta_set("last_refresh", &now_ms().to_string())?;
                inner.store.meta_delete("last_error")?;
                inner.auto_full_text(summary.new_item_ids.clone());
            }
            Err(e) => {
                inner.store.meta_set("last_error", &e.to_string())?;
                inner.platform.log(LogLevel::Warn, &format!("refresh failed: {e}"));
            }
        }
        if let Err(e) = inner.store.evict(self.cache_limit()?) {
            inner.platform.log(LogLevel::Warn, &format!("cache eviction failed: {e}"));
        }
        result
    }

    pub fn sources(&self) -> Result<Vec<SourceInfo>> {
        let counts: std::collections::HashMap<String, u32> = self.inner.store.unread_counts()?.into_iter().collect();
        Ok(self
            .inner
            .store
            .sources()?
            .into_iter()
            .map(|s| SourceInfo { unread: counts.get(&s.id).copied().unwrap_or(0), source: s })
            .collect())
    }

    pub fn items(&self, query: &ItemQuery) -> Result<Vec<ItemRow>> {
        Ok(self.inner.store.items(query)?)
    }

    pub fn item(&self, id: &str) -> Result<Option<ItemRow>> {
        Ok(self.inner.store.item(id)?)
    }

    /// The current document of an item, re-downloading evicted feed content.
    pub async fn document(&self, item_id: &str) -> Result<Option<LoadedDocument>> {
        if let Some(doc) = self.inner.current_document(item_id)? {
            return Ok(Some(doc));
        }
        let row = self.inner.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let direct = self.inner.direct.read().clone();
        if let (Some(direct), Some(up)) = (direct, row.item.upstream_id.as_deref()) {
            if direct.refetch(&self.inner, item_id, up).await? {
                return self.inner.current_document(item_id);
            }
        }
        Ok(None)
    }

    pub async fn set_read(&self, item_id: &str, read: bool) -> Result<()> {
        self.set_item_flag(item_id, fields::READ, read).await
    }

    pub async fn set_starred(&self, item_id: &str, starred: bool) -> Result<()> {
        self.set_item_flag(item_id, fields::STARRED, starred).await
    }

    async fn set_item_flag(&self, item_id: &str, field: &str, value: bool) -> Result<()> {
        let row = self.inner.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let current = if field == fields::READ { row.item.read } else { row.item.starred };
        if current == value {
            return Ok(());
        }
        let upstream_pending = row.item.upstream_id.is_some();
        self.inner.record(Entity::Item, item_id, field, serde_json::Value::Bool(value), upstream_pending)?;
        self.inner.push_in_background();
        Ok(())
    }

    /// Image bytes and MIME type, cached for offline reading.
    pub async fn image(&self, url: &str) -> Result<(Vec<u8>, String)> {
        self.inner.image(url).await
    }

    pub fn cache_limit(&self) -> Result<u64> {
        Ok(self
            .inner
            .store
            .meta_get("cache_limit")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(self.inner.options.default_cache_limit))
    }

    pub fn set_cache_limit(&self, bytes: u64) -> Result<()> {
        self.inner.store.meta_set("cache_limit", &bytes.to_string())?;
        self.inner.store.evict(bytes)?;
        Ok(())
    }

    pub fn cache_usage(&self) -> Result<u64> {
        Ok(self.inner.store.blob_usage()?)
    }

    pub fn set_source_full_text(&self, source_id: &str, mode: FullTextMode) -> Result<()> {
        self.inner.record(Entity::Source, source_id, fields::FULL_TEXT, mode.as_str().into(), false)
    }
}

impl Inner {
    fn restore_upstream(&self) -> Result<()> {
        if let Some(account) = self.store.accounts()?.into_iter().find(|a| a.kind == "greader") {
            if let Some(password) = self.platform.secret_get(&account_secret_key(&account.id)) {
                let client = GReaderClient::new(Credentials {
                    base_url: account.base_url.clone(),
                    username: account.username.clone(),
                    password,
                })?;
                self.set_direct(DirectUpstream { account, client });
            }
        }
        Ok(())
    }

    fn set_direct(&self, direct: DirectUpstream) {
        let direct = Arc::new(direct);
        *self.direct.write() = Some(direct.clone());
        *self.upstream.write() = Some(direct);
    }

    /// Records a local change as an op and applies it.
    pub(crate) fn record(
        &self,
        entity: Entity,
        entity_id: &str,
        field: &str,
        value: serde_json::Value,
        upstream_pending: bool,
    ) -> Result<()> {
        let op = Op {
            op_id: ids::new_local_id(),
            hlc: self.clock.now(),
            entity,
            entity_id: entity_id.to_string(),
            field: field.to_string(),
            value,
        };
        let direct = self.upstream.read().as_ref().is_some_and(|u| u.kind() == "direct");
        self.store.record_local(&op, upstream_pending && direct)?;
        Ok(())
    }

    pub(crate) fn push_in_background(self: &Arc<Self>) {
        let Some(upstream) = self.upstream.read().clone() else { return };
        let inner = self.clone();
        tokio::spawn(async move {
            if let Err(e) = upstream.push(&inner).await {
                inner.platform.log(LogLevel::Warn, &format!("push failed: {e}"));
            }
        });
    }

    /// Hook run after new content is stored (search indexing, AI triggers).
    pub(crate) fn after_ingest(&self, _item_id: &str, _ingested: &Ingested) {}
}
