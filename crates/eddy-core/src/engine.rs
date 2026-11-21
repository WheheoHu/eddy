use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eddy_fetch::{Fetcher, WebRenderer};
use eddy_model::{
    Account, Entity, FullTextMode, HlcClock, Op, Source, SourceKind, fields, ids, now_ms,
};
use eddy_source_greader::{Credentials, GReaderClient, normalize_base};
use eddy_store::{ItemFilter, ItemQuery, ItemRow, Store};
use parking_lot::RwLock;

use crate::content::{Ingested, LoadedDocument};
use crate::platform::{LogLevel, Platform, account_secret_key};
use crate::upstream::{DirectUpstream, Upstream};
use crate::{EngineError, Result};

#[derive(Clone, Debug)]
pub struct EngineOptions {
    /// Cache limit used until the user sets one (bytes).
    pub default_cache_limit: u64,
    /// Running inside eddy-server: everything pulled from FreshRSS is
    /// published as ops for clients, and client-side processing (full text,
    /// AI) is off.
    pub server_role: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            default_cache_limit: 2 * 1024 * 1024 * 1024,
            server_role: false,
        }
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
    /// Server mode: false once a request failed to reach the server, until
    /// one gets through again. Always true in the other modes.
    pub server_online: bool,
    /// Local changes waiting to be uploaded to the server.
    pub pending_uploads: u32,
    /// Changes the server refused for good (shown so they are not silent).
    pub rejected_uploads: u32,
    /// Server mode: FreshRSS credentials are on this device, so new articles
    /// still arrive while the server is offline.
    pub offline_refresh: bool,
    /// A sync with the server that is large enough to be worth showing.
    pub sync: Option<SyncProgress>,
}

/// How far a long sync with the server has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncProgress {
    pub phase: SyncPhase,
    pub done: u64,
    /// Unknown against a server that does not say how much it has.
    pub total: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    /// Sending this device's changes.
    Uploading,
    /// Receiving and applying the server's changes.
    Downloading,
    /// Downloading articles to read offline.
    FetchingContent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshSummary {
    pub new_items: u32,
    pub state_changes: u32,
    pub sources: u32,
    /// IDs of the newly ingested items.
    pub new_item_ids: Vec<String>,
    /// The server was unreachable: new articles came straight from FreshRSS.
    pub offline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceStats {
    pub source_id: String,
    pub total: u32,
    pub unread: u32,
    pub latest_published_at: Option<i64>,
    pub last_read_at: Option<i64>,
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
    pub pdf_reader: RwLock<Option<Arc<dyn crate::PdfReader>>>,
    pub progress_throttle:
        parking_lot::Mutex<std::collections::HashMap<String, (std::time::Instant, f64)>>,
    /// Items whose translation should stop after the batch in flight.
    pub translation_cancels: parking_lot::Mutex<std::collections::HashSet<String>>,
    /// Block being read per item; its translation batch goes first.
    pub translation_focus: parking_lot::Mutex<std::collections::HashMap<String, String>>,
    /// Caps concurrent translation requests; sized by `translation_provider`.
    pub translation_gate: parking_lot::Mutex<(usize, Arc<tokio::sync::Semaphore>)>,
    /// Caps concurrent summary/tagging requests; sized by `summary_provider`.
    pub summary_gate: parking_lot::Mutex<(usize, Arc<tokio::sync::Semaphore>)>,
    /// One request per AI cache key at a time; whoever waits finds it cached.
    pub ai_requests:
        parking_lot::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Items whose full text was fetched or tried in the background this
    /// session; opening them doesn't try again.
    pub full_text_tried: parking_lot::Mutex<std::collections::HashSet<String>>,
    pub search: Option<eddy_search::SearchIndex>,
    /// The engine itself, for background tasks started from `&self`.
    pub me: std::sync::Weak<Inner>,
    /// The live-sync task, if running.
    pub live: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// When the server last failed to answer; `None` while it is reachable.
    pub hub_down_since: parking_lot::Mutex<Option<std::time::Instant>>,
    /// A background upload is waiting to run.
    pub push_scheduled: std::sync::atomic::AtomicBool,
    /// One upload to the server at a time.
    pub push_lock: tokio::sync::Mutex<()>,
    /// Called when a background sync changed local data.
    pub change_listener: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Progress of the sync in flight, and when listeners last heard of it.
    pub sync_progress: parking_lot::Mutex<(Option<SyncProgress>, Option<std::time::Instant>)>,
    /// Called when `sync_progress` moved.
    pub progress_listener: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// When a sync in flight last told the change listener about new data.
    pub sync_notified: parking_lot::Mutex<Option<std::time::Instant>>,
    pub backfilling: std::sync::atomic::AtomicBool,
    pub media: eddy_media::Resolver,
    /// Caps concurrent media lookups across the grid and prefetching.
    pub media_gate: tokio::sync::Semaphore,
    pub media_pending: parking_lot::Mutex<std::collections::HashSet<String>>,
    /// PDFs whose figures are being rendered into the image cache.
    pub pdf_warming: parking_lot::Mutex<std::collections::HashSet<String>>,
    /// Caps how many sites are asked for their icon at once.
    pub icon_gate: tokio::sync::Semaphore,
    /// Image hosts that refused a request without a `Referer` and served it
    /// with one; the rest of this run asks them that way the first time.
    pub referer_hosts: parking_lot::Mutex<std::collections::HashSet<String>>,
    pub video_cancels:
        parking_lot::Mutex<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    pub video_listener: RwLock<Option<crate::video::VideoJobCallback>>,
    pub export_cancels: parking_lot::Mutex<std::collections::HashSet<String>>,
    pub export_translating: parking_lot::Mutex<std::collections::HashMap<String, String>>,
}

/// Cheap to clone; all clones share one engine.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl Engine {
    pub fn open(
        data_dir: &Path,
        platform: Arc<dyn Platform>,
        options: EngineOptions,
    ) -> Result<Engine> {
        // A device that signed out is erased before anything is opened.
        crate::erase::finish_pending(data_dir, platform.as_ref())?;
        let store = Store::open(data_dir)?;
        let device_id = store.device_id()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(15))
            .http2_max_header_list_size(eddy_fetch::MAX_HEADER_LIST_BYTES)
            .user_agent(concat!(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Eddy/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let fetcher = Fetcher::new(Default::default(), None)
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let inner = Arc::new_cyclic(|me| Inner {
            me: me.clone(),
            store,
            platform,
            clock: HlcClock::new(device_id),
            http,
            upstream: RwLock::new(None),
            direct: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
            data_dir: data_dir.to_path_buf(),
            options,
            fetcher: RwLock::new(Arc::new(fetcher)),
            web_renderer: RwLock::new(None),
            pdf_reader: RwLock::new(None),
            progress_throttle: parking_lot::Mutex::new(Default::default()),
            translation_cancels: parking_lot::Mutex::new(Default::default()),
            translation_focus: parking_lot::Mutex::new(Default::default()),
            translation_gate: parking_lot::Mutex::new((
                1,
                Arc::new(tokio::sync::Semaphore::new(1)),
            )),
            summary_gate: parking_lot::Mutex::new((1, Arc::new(tokio::sync::Semaphore::new(1)))),
            ai_requests: parking_lot::Mutex::new(Default::default()),
            full_text_tried: parking_lot::Mutex::new(Default::default()),
            search: eddy_search::SearchIndex::open(&data_dir.join("index")).ok(),
            live: parking_lot::Mutex::new(None),
            hub_down_since: parking_lot::Mutex::new(None),
            push_scheduled: std::sync::atomic::AtomicBool::new(false),
            push_lock: tokio::sync::Mutex::new(()),
            change_listener: RwLock::new(None),
            sync_progress: parking_lot::Mutex::new((None, None)),
            progress_listener: RwLock::new(None),
            sync_notified: parking_lot::Mutex::new(None),
            backfilling: std::sync::atomic::AtomicBool::new(false),
            media: eddy_media::Resolver::new(),
            media_gate: tokio::sync::Semaphore::new(4),
            media_pending: parking_lot::Mutex::new(Default::default()),
            pdf_warming: parking_lot::Mutex::new(Default::default()),
            icon_gate: tokio::sync::Semaphore::new(4),
            referer_hosts: parking_lot::Mutex::new(Default::default()),
            video_cancels: parking_lot::Mutex::new(Default::default()),
            video_listener: RwLock::new(None),
            export_cancels: parking_lot::Mutex::new(Default::default()),
            export_translating: parking_lot::Mutex::new(Default::default()),
        });
        let _ = inner.store.recover_interrupted_video_jobs(now_ms());
        let _ = inner.migrate_ai_keys();
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
            gallery: false,
        })?;
        // After a restart the clock continues past everything in the log.
        if let Some(latest) = inner.store.max_hlc()? {
            inner.clock.observe(&latest);
        }
        // The FreshRSS account stays available in server mode, for refreshing
        // while the server is offline.
        inner.restore_direct()?;
        if !inner.restore_hub()?
            && let Some(direct) = inner.direct.read().clone()
        {
            *inner.upstream.write() = Some(direct);
        }
        if !inner.options.server_role {
            match inner.restore_feed_versions() {
                Ok(0) => {}
                Ok(n) => inner.platform.log(
                    LogLevel::Info,
                    &format!("restored feed content of {n} items"),
                ),
                Err(e) => inner.platform.log(
                    LogLevel::Warn,
                    &format!("restoring feed content failed: {e}"),
                ),
            }
        }
        Ok(Engine { inner })
    }

    pub fn device_id(&self) -> String {
        self.inner.clock.node().to_string()
    }

    /// Installs a callback run after a background sync (live channel) changed
    /// local data, so open lists can reload.
    pub fn set_change_listener(&self, listener: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self.inner.change_listener.write() = listener;
    }

    /// Runs the change listener: for a host that changed data on someone
    /// else's behalf (the command line working through the app) and wants
    /// open lists to reload.
    pub fn notify_changed(&self) {
        self.inner.notify_changed();
    }

    /// Called as a long sync with the server moves; read the new state from
    /// `status().sync`.
    pub fn set_progress_listener(&self, listener: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self.inner.progress_listener.write() = listener;
    }

    pub fn status(&self) -> Result<EngineStatus> {
        let inner = &self.inner;
        let account = inner.direct.read().as_ref().map(|d| d.account.clone());
        let mode = match inner.upstream.read().as_ref().map(|u| u.kind()) {
            Some("direct") => Mode::Standalone,
            Some(_) => Mode::Server,
            None => Mode::Unconfigured,
        };
        let (pending_uploads, rejected_uploads) = if mode == Mode::Server {
            inner.store.upload_counts()?
        } else {
            (0, 0)
        };
        Ok(EngineStatus {
            server_online: mode != Mode::Server || inner.hub_down_since.lock().is_none(),
            pending_uploads,
            rejected_uploads,
            offline_refresh: mode == Mode::Server && account.is_some(),
            mode,
            account,
            last_refresh: inner
                .store
                .meta_get("last_refresh")?
                .and_then(|v| v.parse().ok()),
            last_error: inner.store.meta_get("last_error")?,
            sync: inner.sync_progress.lock().0,
        })
    }

    /// Verifies the FreshRSS credentials, stores the password in the platform
    /// keychain and switches to standalone mode.
    pub async fn configure_freshrss(
        &self,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Account> {
        let base = normalize_base(url)?;
        let client = GReaderClient::new(Credentials {
            base_url: base.clone(),
            username: username.into(),
            password: password.into(),
        })?;
        client.login().await?;
        client.subscriptions().await?;
        let account = Account {
            id: ids::account_id("greader", &base, username.trim()),
            kind: "greader".into(),
            base_url: base,
            username: username.trim().to_string(),
            created_at: now_ms(),
        };
        // In server mode this only enables refreshing while the server is
        // offline, so it has to be the account the server manages: the same
        // articles then get the same IDs on both sides.
        let hub = self.inner.is_hub();
        if hub
            && self
                .inner
                .store
                .meta_get(crate::hub::SERVER_ACCOUNT_META)?
                .is_some_and(|id| id != account.id)
        {
            return Err(EngineError::Invalid(
                "这不是服务端使用的 FreshRSS 账号。离线刷新需要填写同一个地址和用户名。".into(),
            ));
        }
        if !self
            .inner
            .platform
            .secret_set(&account_secret_key(&account.id), password)
        {
            return Err(EngineError::Invalid("无法把密码保存到钥匙串。".into()));
        }
        for old in self.inner.store.accounts()? {
            if old.id != account.id {
                self.inner
                    .platform
                    .secret_delete(&account_secret_key(&old.id));
                self.inner.store.delete_account(&old.id)?;
            }
        }
        self.inner.store.upsert_account(&account)?;
        let direct = Arc::new(DirectUpstream {
            account: account.clone(),
            client,
        });
        *self.inner.direct.write() = Some(direct.clone());
        if !hub {
            *self.inner.upstream.write() = Some(direct);
        }
        Ok(account)
    }

    pub fn remove_account(&self) -> Result<()> {
        for account in self.inner.store.accounts()? {
            self.inner
                .platform
                .secret_delete(&account_secret_key(&account.id));
            self.inner.store.delete_account(&account.id)?;
        }
        *self.inner.direct.write() = None;
        if !self.inner.is_hub() {
            *self.inner.upstream.write() = None;
        }
        Ok(())
    }

    /// Syncs with the active upstream and trims the cache.
    pub async fn refresh(&self) -> Result<RefreshSummary> {
        let inner = &self.inner;
        let _guard = inner.refresh_lock.lock().await;
        let upstream = inner
            .upstream
            .read()
            .clone()
            .ok_or(EngineError::NotConfigured)?;
        let mut result = upstream.sync(inner).await;
        if upstream.kind() == "hub" {
            inner.note_hub(&result);
            // The server is the usual way in for new articles; while it is
            // unreachable they come straight from FreshRSS.
            let direct = inner.direct.read().clone();
            if matches!(result, Err(EngineError::Offline(_)))
                && let Some(direct) = direct
            {
                match direct.pull_new(inner).await {
                    Ok(summary) => result = Ok(summary),
                    Err(e) => inner
                        .platform
                        .log(LogLevel::Warn, &format!("offline refresh failed: {e}")),
                }
            }
        }
        match &result {
            Ok(summary) => {
                inner
                    .store
                    .meta_set("last_refresh", &now_ms().to_string())?;
                inner.store.meta_delete("last_error")?;
                if !inner.options.server_role {
                    inner.auto_process(summary.new_item_ids.clone());
                }
            }
            Err(e) => {
                inner.store.meta_set("last_error", &e.to_string())?;
                inner
                    .platform
                    .log(LogLevel::Warn, &format!("refresh failed: {e}"));
            }
        }
        self.settle()?;
        result
    }

    /// Trims the cache and makes newly stored content searchable.
    pub(crate) fn settle(&self) -> Result<()> {
        if let Err(e) = self.inner.store.evict(self.cache_limit()?) {
            self.inner
                .platform
                .log(LogLevel::Warn, &format!("cache eviction failed: {e}"));
        }
        self.inner.commit_index();
        Ok(())
    }

    /// Runs `f` with this thread's reads on a connection of their own, so
    /// that a long count does not keep other callers waiting for the store.
    /// For reading only: a write inside `f` fails.
    pub fn read_aside<T>(&self, f: impl FnOnce() -> T) -> T {
        self.inner.store.read_aside(f)
    }

    pub fn sources(&self) -> Result<Vec<SourceInfo>> {
        let counts: std::collections::HashMap<String, u32> =
            self.inner.store.unread_counts()?.into_iter().collect();
        Ok(self
            .inner
            .store
            .sources()?
            .into_iter()
            .map(|s| SourceInfo {
                unread: counts.get(&s.id).copied().unwrap_or(0),
                source: s,
            })
            .collect())
    }

    /// Per-source totals for library views.
    pub fn source_overview(&self) -> Result<Vec<SourceStats>> {
        Ok(self
            .inner
            .store
            .source_overview()?
            .into_iter()
            .map(
                |(source_id, total, unread, latest_published_at, last_read_at)| SourceStats {
                    source_id,
                    total,
                    unread,
                    latest_published_at,
                    last_read_at,
                },
            )
            .collect())
    }

    pub fn items(&self, query: &ItemQuery) -> Result<Vec<ItemRow>> {
        Ok(self.inner.store.items(query)?)
    }

    /// How many items `items` would list without its limit and offset.
    pub fn item_count(&self, query: &ItemQuery) -> Result<u32> {
        Ok(self.inner.store.count_items(query)?)
    }

    pub fn item(&self, id: &str) -> Result<Option<ItemRow>> {
        Ok(self.inner.store.item(id)?)
    }

    /// The current document of an item, re-downloading evicted feed content.
    pub async fn document(&self, item_id: &str) -> Result<Option<LoadedDocument>> {
        if let Some(doc) = self.inner.current_document(item_id)? {
            // Opening a PDF (first time, or after its document was rebuilt)
            // starts its figures rendering, so they come up as it is read.
            self.inner.warm_pdf_images(&doc.document);
            return Ok(Some(doc));
        }
        // An unreachable server is not the end: the page or FreshRSS may
        // still have the content.
        match self.fetch_missing_blobs(item_id).await {
            Ok(true) => {
                if let Some(doc) = self.inner.current_document(item_id)? {
                    return Ok(Some(doc));
                }
            }
            Ok(false) => {}
            Err(e) => self
                .inner
                .platform
                .log(LogLevel::Warn, &format!("content download failed: {e}")),
        }
        if self.inner.ensure_page_content(item_id).await? {
            return self.inner.current_document(item_id);
        }
        let row = self
            .inner
            .store
            .item(item_id)?
            .ok_or(EngineError::NotFound)?;
        let direct = self.inner.direct.read().clone();
        if let (Some(direct), Some(up)) = (direct, row.item.upstream_id.as_deref())
            && direct.refetch(&self.inner, item_id, up).await?
        {
            return self.inner.current_document(item_id);
        }
        Ok(None)
    }

    pub async fn set_read(&self, item_id: &str, read: bool) -> Result<()> {
        self.set_item_flag(item_id, fields::READ, read).await
    }

    /// How many articles `mark_all_read` with the same arguments would mark.
    pub fn unread_feed_count(&self, filter: &ItemFilter, until: i64) -> Result<u32> {
        Ok(self
            .inner
            .store
            .unread_feed_ids(&Self::list(filter), until)?
            .len() as u32)
    }

    /// Marks read the unread subscription articles a list holds that arrived
    /// by `until` (ms): not the ones that come in while the user decides,
    /// nor saved pages. Reading progress is left alone. Returns how many.
    pub fn mark_all_read(&self, filter: &ItemFilter, until: i64) -> Result<u32> {
        let inner = &self.inner;
        let ids = inner.store.unread_feed_ids(&Self::list(filter), until)?;
        let mut upstream = false;
        for id in &ids {
            let pending = inner
                .store
                .item(id)?
                .is_some_and(|r| r.item.upstream_id.is_some());
            upstream |= pending;
            inner.record(
                Entity::Item,
                id,
                fields::READ,
                serde_json::Value::Bool(true),
                pending,
            )?;
        }
        if upstream {
            inner.push_in_background();
        }
        Ok(ids.len() as u32)
    }

    fn list(filter: &ItemFilter) -> ItemQuery {
        ItemQuery {
            filter: filter.clone(),
            unread_only: true,
            keep: Vec::new(),
            limit: u32::MAX,
            offset: 0,
        }
    }

    pub async fn set_starred(&self, item_id: &str, starred: bool) -> Result<()> {
        self.set_item_flag(item_id, fields::STARRED, starred).await
    }

    async fn set_item_flag(&self, item_id: &str, field: &str, value: bool) -> Result<()> {
        let row = self
            .inner
            .store
            .item(item_id)?
            .ok_or(EngineError::NotFound)?;
        let current = if field == fields::READ {
            row.item.read
        } else {
            row.item.starred
        };
        if current == value {
            return Ok(());
        }
        let upstream_pending = row.item.upstream_id.is_some();
        self.inner.record(
            Entity::Item,
            item_id,
            field,
            serde_json::Value::Bool(value),
            upstream_pending,
        )?;
        self.inner.push_in_background();
        Ok(())
    }

    /// Image bytes and MIME type, cached for offline reading. `page` is the
    /// article the image belongs to, named to hosts that refuse to serve
    /// pictures to requests that come from nowhere.
    pub async fn image(&self, url: &str, page: Option<&str>) -> Result<(Vec<u8>, String)> {
        self.inner.image(url, page).await
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
        self.inner
            .store
            .meta_set("cache_limit", &bytes.to_string())?;
        self.inner.store.evict(bytes)?;
        Ok(())
    }

    pub fn cache_usage(&self) -> Result<u64> {
        Ok(self.inner.store.blob_usage()?)
    }

    /// Empties the offline cache now, whatever its limit: everything the
    /// limit would let go of first. Unread, starred, saved and in-progress
    /// content and imported files stay. Returns the bytes freed.
    pub fn clear_cache(&self) -> Result<u64> {
        Ok(self.inner.store.evict(0)?)
    }
}

impl Inner {
    pub(crate) fn is_hub(&self) -> bool {
        self.upstream
            .read()
            .as_ref()
            .is_some_and(|u| u.kind() == "hub")
    }

    /// Records whether a request reached the server.
    pub(crate) fn note_hub<T>(&self, result: &Result<T>) {
        let mut down = self.hub_down_since.lock();
        match result {
            Err(EngineError::Offline(_)) => *down = Some(std::time::Instant::now()),
            _ => *down = None,
        }
    }

    /// Uploads pending changes shortly, retrying with backoff while the
    /// server is unreachable. Calls made while one is waiting join it.
    pub(crate) fn schedule_push(&self) {
        use std::sync::atomic::Ordering;
        if self.options.server_role || !self.is_hub() {
            return;
        }
        let (Some(inner), Ok(runtime)) = (self.me.upgrade(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        if self.push_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        runtime.spawn(async move {
            let mut delay = Duration::from_millis(500);
            loop {
                tokio::time::sleep(delay).await;
                // Changes recorded from here on schedule their own upload.
                inner.push_scheduled.store(false, Ordering::SeqCst);
                let upstream = inner.upstream.read().clone();
                let Some(upstream) = upstream.filter(|u| u.kind() == "hub") else {
                    break;
                };
                let result = upstream.push(&inner).await;
                inner.note_hub(&result);
                let Err(e) = result else { break };
                inner
                    .platform
                    .log(LogLevel::Warn, &format!("push failed: {e}"));
                // Only an unreachable server is worth waiting for.
                if !matches!(e, EngineError::Offline(_))
                    || inner.push_scheduled.swap(true, Ordering::SeqCst)
                {
                    break;
                }
                delay = (delay * 2).clamp(Duration::from_secs(5), Duration::from_secs(300));
            }
        });
    }

    /// Publishes how far the sync in flight is (`None`: finished). Listeners
    /// hear of a new phase and of the end at once, and of steps within a
    /// phase a few times a second.
    pub(crate) fn set_sync_progress(&self, progress: Option<SyncProgress>) {
        {
            let mut state = self.sync_progress.lock();
            if state.0 == progress {
                return;
            }
            let same_phase =
                matches!((&state.0, &progress), (Some(a), Some(b)) if a.phase == b.phase);
            state.0 = progress;
            if same_phase
                && state
                    .1
                    .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
            {
                return;
            }
            state.1 = Some(std::time::Instant::now());
        }
        if let Some(listener) = self.progress_listener.read().clone() {
            listener();
        }
    }

    /// During a long sync: lets lists pick up what has arrived so far, at
    /// most every couple of seconds.
    pub(crate) fn notify_changed_during_sync(&self) {
        {
            let mut last = self.sync_notified.lock();
            if last.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        self.notify_changed();
    }

    pub(crate) fn notify_changed(&self) {
        if let Some(listener) = self.change_listener.read().clone() {
            listener();
        }
    }

    fn restore_direct(&self) -> Result<()> {
        if let Some(account) = self
            .store
            .accounts()?
            .into_iter()
            .find(|a| a.kind == "greader")
            && let Some(password) = self.platform.secret_get(&account_secret_key(&account.id))
        {
            let client = GReaderClient::new(Credentials {
                base_url: account.base_url.clone(),
                username: account.username.clone(),
                password,
            })?;
            *self.direct.write() = Some(Arc::new(DirectUpstream { account, client }));
        }
        Ok(())
    }

    /// On a server, records an op so clients receive a change that was
    /// applied directly (items and sources from FreshRSS). No-op on clients.
    pub(crate) fn publish(
        &self,
        entity: Entity,
        entity_id: &str,
        field: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        if self.options.server_role {
            self.record(entity, entity_id, field, value, false)?;
        }
        Ok(())
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
        let direct = self
            .upstream
            .read()
            .as_ref()
            .is_some_and(|u| u.kind() == "direct");
        self.store.record_local(&op, upstream_pending && direct)?;
        // Every change reaches the server on its own, not only at the next
        // refresh.
        self.schedule_push();
        Ok(())
    }

    pub(crate) fn push_in_background(self: &Arc<Self>) {
        let Some(upstream) = self.upstream.read().clone() else {
            return;
        };
        if upstream.kind() == "hub" {
            return self.schedule_push();
        }
        let inner = self.clone();
        tokio::spawn(async move {
            if let Err(e) = upstream.push(&inner).await {
                inner
                    .platform
                    .log(LogLevel::Warn, &format!("push failed: {e}"));
            }
        });
    }

    /// Hook run after new content is stored.
    pub(crate) fn after_ingest(&self, item_id: &str, ingested: &Ingested) {
        self.index_item(item_id, ingested);
    }
}
