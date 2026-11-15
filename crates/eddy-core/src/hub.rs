use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eddy_model::{ContentVersion, Entity, Hlc, Op, fields, ids};
use eddy_sync::{AccountRequest, PairRequest, ServerInfo, SyncClient, SyncError};

use crate::engine::{Engine, Inner, RefreshSummary, SyncPhase, SyncProgress};
use crate::platform::account_secret_key;
use crate::search::SearchResult;
use crate::upstream::Upstream;
use crate::{EngineError, LogLevel, Result};

const TOKEN_KEY: &str = "server:token";
const URL_META: &str = "server_url";
const CURSOR_META: &str = "sync_cursor";
/// The server this device last synced with. A different one has none of
/// this device's history, whatever the local bookkeeping says.
const SERVER_ID_META: &str = "server_id";
/// Protocol the server spoke last time; a newer one may accept ops the old
/// one refused.
const SERVER_PROTOCOL_META: &str = "server_protocol";
/// ID of the FreshRSS account the server manages.
pub(crate) const SERVER_ACCOUNT_META: &str = "server_account";
/// Set when a pull skipped ops this version cannot read; the next version
/// pulls the log again to pick them up.
const SKIPPED_META: &str = "sync_skipped";
/// Protocol version of the build that last synced this data.
const CLIENT_PROTOCOL_META: &str = "sync_protocol";
const ARTIFACT_BATCH: usize = 100;
const ARTIFACT_BATCH_BYTES: usize = 8 * 1024 * 1024;
/// After a failed request the server counts as offline this long before
/// interactive features (search, video jobs) try it again.
const OFFLINE_GRACE: Duration = Duration::from_secs(60);
const PUSH_BATCH: u32 = 500;
/// The most a server hands out in one page; it also cuts a page short by
/// size.
const PULL_BATCH: u32 = 5000;
const PREFETCH_CAP: usize = 200;
const PREFETCH_CONCURRENCY: usize = 4;
/// Fewer downloads than this finish before progress would be read.
const PREFETCH_SHOWN_FROM: usize = 20;

impl From<SyncError> for EngineError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Network(message) => EngineError::Offline(message),
            // A proxy in front of a stopped server answers like this.
            SyncError::Http(502..=504, _) => EngineError::Offline(e.to_string()),
            SyncError::Unauthorized => EngineError::Unauthorized,
            SyncError::Upgrade(message) => EngineError::UpgradeRequired(message),
            other => EngineError::Network(other.to_string()),
        }
    }
}

/// Server mode: syncs with eddy-server, which owns the FreshRSS account.
pub(crate) struct HubUpstream {
    pub client: SyncClient,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerStatus {
    pub url: String,
    pub server_id: String,
    pub account: Option<eddy_sync::AccountInfo>,
    pub cursor: i64,
    pub video_capable: bool,
    /// The server's software version.
    pub version: String,
}

impl HubUpstream {
    /// Uploads the blobs of content versions this device created, so other
    /// devices can open them.
    async fn upload_blobs(&self, inner: &Inner, ops: &[eddy_store::StoredOp]) -> Result<()> {
        for stored in ops
            .iter()
            .filter(|o| o.op.entity == Entity::ContentVersion && o.op.field == fields::CREATE)
        {
            let Ok(version) = serde_json::from_value::<ContentVersion>(stored.op.value.clone())
            else {
                continue;
            };
            for key in [Some(&version.raw_blob), version.doc_blob.as_ref()]
                .into_iter()
                .flatten()
            {
                if self.client.has_blob(key).await? {
                    continue;
                }
                if let Some(bytes) = inner.store.get_blob(key)? {
                    let mime = inner
                        .store
                        .blob_mime(key)?
                        .unwrap_or_else(|| "application/octet-stream".into());
                    self.client.put_blob(key, bytes, &mime).await?;
                }
            }
        }
        Ok(())
    }

    /// Synced AI results travel as artifact ops generated at upload time,
    /// in batches small enough for one request.
    async fn upload_artifacts(&self, inner: &Inner) -> Result<()> {
        let mut artifacts = inner
            .store
            .unuploaded_ai_artifacts()?
            .into_iter()
            .peekable();
        while artifacts.peek().is_some() {
            let mut ops = Vec::new();
            let mut keys = Vec::new();
            let mut bytes = 0;
            while let Some((key, item_id, kind, content_hash, lang, model, payload)) = artifacts
                .next_if(|a| {
                    ops.is_empty()
                        || (ops.len() < ARTIFACT_BATCH && bytes + a.6.len() <= ARTIFACT_BATCH_BYTES)
                })
            {
                bytes += payload.len();
                ops.push(Op {
                    op_id: ids::new_local_id(),
                    hlc: inner.clock.now(),
                    entity: Entity::Artifact,
                    entity_id: key.clone(),
                    field: fields::CREATE.into(),
                    value: serde_json::json!({ "item_id": item_id, "kind": kind, "content_hash": content_hash, "lang": lang, "model": model, "payload": payload }),
                });
                keys.push(key);
            }
            self.client.push(ops).await?;
            inner.store.mark_ai_artifacts_uploaded(&keys)?;
        }
        Ok(())
    }

    /// Checks the server is one this build can sync with, and notices when
    /// it is not the server this device synced with before.
    async fn handshake(&self, inner: &Inner) -> Result<()> {
        let info = self.client.info().await?;
        info.check_protocol()?;
        inner.adopt_server(&info)
    }

    /// Two-way union of private rule packs: local packs the server lacks (or
    /// has with different content) go up, server packs missing here come down.
    async fn sync_rule_packs(&self, inner: &Inner) -> Result<()> {
        let remote = self.client.config().await?;
        let dir = inner.rules_dir();
        let mut changed = false;
        let local: Vec<(String, String)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .filter_map(|p| {
                Some((
                    p.file_stem()?.to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).ok()?,
                ))
            })
            .collect();
        // A pack removed on any device stays on the server as an empty
        // entry, so it is removed everywhere instead of being uploaded again
        // by the devices that still have it.
        let removed_here = inner.removed_rule_packs();
        for name in &removed_here {
            let key = format!("rulepack:{name}");
            if remote.iter().any(|e| e.key == key && !e.value.is_empty()) {
                self.client.put_config(&key, "").await?;
            }
        }
        inner.clear_removed_rule_packs()?;
        // Only packs this device already shared with the server follow a
        // removal; one imported since is new and goes up instead.
        let shared = inner.shared_rule_packs();
        let mut local = local;
        local.retain(|(name, _)| {
            let key = format!("rulepack:{name}");
            let gone =
                shared.contains(name) && remote.iter().any(|e| e.key == key && e.value.is_empty());
            if gone {
                let _ = std::fs::remove_file(dir.join(format!("{name}.toml")));
                changed = true;
            }
            !gone
        });
        for (name, text) in &local {
            let key = format!("rulepack:{name}");
            if remote
                .iter()
                .find(|e| e.key == key)
                .is_none_or(|e| &e.value != text)
            {
                self.client.put_config(&key, text).await?;
            }
        }
        for entry in remote {
            let Some(name) = entry.key.strip_prefix("rulepack:") else {
                continue;
            };
            if entry.value.is_empty()
                || removed_here.iter().any(|n| n == name)
                || local.iter().any(|(n, _)| n == name)
                || !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                continue;
            }
            std::fs::create_dir_all(&dir).map_err(|e| EngineError::Invalid(e.to_string()))?;
            std::fs::write(dir.join(format!("{name}.toml")), &entry.value)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
            local.push((name.to_string(), entry.value.clone()));
            changed = true;
        }
        let names: Vec<&String> = local.iter().map(|(name, _)| name).collect();
        inner.set_shared_rule_packs(&names)?;
        if changed {
            inner.rebuild_fetcher()?;
        }
        Ok(())
    }

    /// Downloads document blobs for content that should be readable offline:
    /// unread, starred, saved or in progress.
    async fn prefetch(&self, inner: &Inner, versions: Vec<String>) {
        use futures::StreamExt;
        // Newest first: after a long catch-up the cap is spent on what the
        // reader is likeliest to open.
        let wanted: Vec<(String, String)> = versions
            .into_iter()
            .rev()
            .filter_map(|hash| {
                let version = inner.store.content_version(&hash).ok()??;
                let row = inner.store.item(&version.item_id).ok()??;
                let wanted = !row.item.read
                    || row.item.starred
                    || row.item.kind != eddy_model::ItemKind::Feed
                    || (row.item.progress > 0.0 && !row.item.finished);
                if !wanted || row.item.current_version.as_deref() != Some(hash.as_str()) {
                    return None;
                }
                let key = version.doc_blob?;
                (!inner.store.has_blob(&key).unwrap_or(true)).then_some((key, version.item_id))
            })
            .take(PREFETCH_CAP)
            .collect();
        let total = wanted.len();
        let report = |done: usize| {
            if total >= PREFETCH_SHOWN_FROM {
                inner.set_sync_progress(Some(SyncProgress {
                    phase: SyncPhase::FetchingContent,
                    done: done as u64,
                    total: Some(total as u64),
                }));
            }
        };
        report(0);
        let mut downloads = futures::stream::iter(wanted)
            .map(|(key, item_id)| async move {
                let blob = self.client.get_blob(&key).await;
                (key, item_id, blob)
            })
            .buffer_unordered(PREFETCH_CONCURRENCY);
        let mut done = 0;
        while let Some((key, item_id, blob)) = downloads.next().await {
            if let Ok(Some((bytes, mime))) = blob
                && inner
                    .store
                    .put_blob_keyed(&key, &bytes, &mime, false)
                    .is_ok()
            {
                inner.index_current(&item_id);
            }
            done += 1;
            report(done);
        }
        inner.commit_index();
    }
}

#[async_trait]
impl Upstream for HubUpstream {
    fn kind(&self) -> &'static str {
        "hub"
    }

    async fn push(&self, inner: &Inner) -> Result<()> {
        let _one = inner.push_lock.lock().await;
        // Only an upload of several batches (a first sync, a long time
        // offline) is worth showing.
        let total = u64::from(inner.store.upload_counts()?.0);
        let shown = total > u64::from(PUSH_BATCH);
        let _progress = shown.then(|| ProgressGuard(inner));
        let mut done = 0u64;
        loop {
            if shown {
                inner.set_sync_progress(Some(SyncProgress {
                    phase: SyncPhase::Uploading,
                    done: done.min(total),
                    total: Some(total),
                }));
            }
            let pending = inner.store.unuploaded_ops(PUSH_BATCH)?;
            if pending.is_empty() {
                break;
            }
            done += pending.len() as u64;
            self.upload_blobs(inner, &pending).await?;
            let seqs: Vec<i64> = pending.iter().map(|o| o.seq).collect();
            let response = self
                .client
                .push(pending.into_iter().map(|o| o.op).collect())
                .await?;
            inner.store.mark_uploaded(&seqs)?;
            if !response.rejected.is_empty() {
                // Refused for good: sending them again would only fail again
                // and hold up everything behind them.
                for r in &response.rejected {
                    inner.platform.log(
                        LogLevel::Warn,
                        &format!("server refused change {}: {}", r.op_id, r.reason),
                    );
                }
                let ids: Vec<String> = response.rejected.into_iter().map(|r| r.op_id).collect();
                inner.store.mark_rejected(&ids)?;
            }
        }
        self.upload_artifacts(inner).await
    }

    async fn sync(&self, inner: &Inner) -> Result<RefreshSummary> {
        self.handshake(inner).await?;
        self.push(inner).await?;
        if let Err(e) = self.client.refresh().await {
            inner
                .platform
                .log(LogLevel::Warn, &format!("server refresh failed: {e}"));
        }
        let mut cursor: i64 = inner
            .store
            .meta_get(CURSOR_META)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut new_item_ids = Vec::new();
        let mut versions = Vec::new();
        let mut deleted = Vec::new();
        let mut state_changes = 0;
        let _progress = ProgressGuard(inner);
        let start = cursor;
        let mut pages = 0;
        // The next page downloads while this one is applied, so a long
        // catch-up is not the sum of the network's time and the database's.
        let fetch = |since: i64| {
            let client = self.client.clone();
            AbortOnDrop(tokio::spawn(
                async move { client.pull(since, PULL_BATCH).await },
            ))
        };
        let mut next = fetch(cursor);
        loop {
            let page = (&mut next.0)
                .await
                .map_err(|e| EngineError::Invalid(e.to_string()))??;
            if page.more {
                next = fetch(page.cursor);
            }
            if page.skipped > 0 {
                inner.store.meta_set(SKIPPED_META, "1")?;
            }
            for op in &page.ops {
                inner.clock.observe(&op.hlc);
            }
            // The page and the cursor past it are stored together: a sync
            // that is cut off resumes exactly after the last whole page.
            let outcome = inner
                .store
                .apply_remote_page(&page.ops, Some((CURSOR_META, &page.cursor.to_string())))?;
            for (op, result) in page.ops.iter().zip(outcome) {
                // An article this device already has (it was here before
                // connecting, or came straight from FreshRSS while the server
                // was offline) is not new.
                let (applied, known) = (result.applied, result.known);
                match (op.entity, op.field.as_str()) {
                    // Items the server ingested already read (backfilled
                    // history included) are not new to this device.
                    (Entity::Item, fields::CREATE)
                        if applied && !known && op.value["read"] != true =>
                    {
                        new_item_ids.push(op.entity_id.clone())
                    }
                    (Entity::Item, fields::VERSION) if applied => {
                        versions.push(op.value.as_str().unwrap_or_default().to_string())
                    }
                    (Entity::Item, fields::READ | fields::STARRED) if applied => state_changes += 1,
                    (Entity::Item, fields::DELETED) if applied && op.value == true => {
                        deleted.push(op.entity_id.clone())
                    }
                    _ => {}
                }
            }
            cursor = page.cursor;
            pages += 1;
            if !page.more {
                break;
            }
            // More than one page: show how far it is, and let what has
            // arrived appear instead of waiting for the end.
            inner.set_sync_progress(Some(SyncProgress {
                phase: SyncPhase::Downloading,
                done: (cursor - start).max(0) as u64,
                total: page.head.map(|head| (head - start).max(0) as u64),
            }));
            if pages > 1 {
                inner.notify_changed_during_sync();
            }
        }
        inner.forget_deleted(&deleted);
        if let Err(e) = self.sync_rule_packs(inner).await {
            inner
                .platform
                .log(LogLevel::Warn, &format!("rule pack sync failed: {e}"));
        }
        self.prefetch(inner, versions).await;
        let sources = inner.store.sources()?.len() as u32;
        Ok(RefreshSummary {
            new_items: new_item_ids.len() as u32,
            state_changes,
            sources,
            new_item_ids,
            offline: false,
        })
    }

    async fn fetch_blob(&self, key: &str) -> Result<Option<(Vec<u8>, String)>> {
        Ok(self.client.get_blob(key).await?)
    }
}

impl Inner {
    pub(crate) fn restore_hub(&self) -> Result<bool> {
        let (Some(url), Some(token)) = (
            self.store.meta_get(URL_META)?,
            self.platform.secret_get(TOKEN_KEY),
        ) else {
            return Ok(false);
        };
        let client = SyncClient::new(&url, Some(token))?;
        *self.upstream.write() = Some(Arc::new(HubUpstream { client }));
        Ok(true)
    }

    /// The server client for interactive features that have a local
    /// fallback: `None` right after the server failed to answer, so they do
    /// not wait for another timeout.
    pub(crate) fn reachable_hub_client(&self) -> Option<SyncClient> {
        if self
            .hub_down_since
            .lock()
            .is_some_and(|at| at.elapsed() < OFFLINE_GRACE)
        {
            return None;
        }
        self.hub_client()
    }

    /// Remembers which server this device syncs with. Against a server it
    /// has not synced with before (another one, or the same one wiped) the
    /// whole log is queued again and pulling starts over.
    pub(crate) fn adopt_server(&self, info: &ServerInfo) -> Result<()> {
        let store = &self.store;
        if store.meta_get(SERVER_ID_META)?.as_deref() != Some(info.server_id.as_str()) {
            if store.meta_get(SERVER_ID_META)?.is_some() || store.meta_get(CURSOR_META)?.is_some() {
                self.platform
                    .log(LogLevel::Info, "new server: uploading the full history");
            }
            store.reset_uploaded()?;
            store.meta_delete(CURSOR_META)?;
            store.meta_set(SERVER_ID_META, &info.server_id)?;
        }
        // A newer server may take what an older one refused.
        let protocol = info.protocol.to_string();
        if store.meta_get(SERVER_PROTOCOL_META)?.as_deref() != Some(protocol.as_str()) {
            store.retry_rejected()?;
            store.meta_set(SERVER_PROTOCOL_META, &protocol)?;
        }
        // This build may read ops an older one had to skip.
        let ours = eddy_sync::PROTOCOL_VERSION.to_string();
        if store.meta_get(CLIENT_PROTOCOL_META)?.as_deref() != Some(ours.as_str()) {
            if store.meta_get(SKIPPED_META)?.is_some() {
                store.meta_delete(CURSOR_META)?;
                store.meta_delete(SKIPPED_META)?;
            }
            store.meta_set(CLIENT_PROTOCOL_META, &ours)?;
        }
        match &info.account {
            Some(account) => store.meta_set(SERVER_ACCOUNT_META, &server_account_id(account))?,
            None => store.meta_delete(SERVER_ACCOUNT_META)?,
        }
        Ok(())
    }

    pub(crate) fn hub_client(&self) -> Option<SyncClient> {
        let url = self.store.meta_get(URL_META).ok().flatten()?;
        let token = self.platform.secret_get(TOKEN_KEY)?;
        SyncClient::new(&url, Some(token)).ok()
    }

    /// Indexes an item's current document if it is available locally.
    pub(crate) fn index_current(&self, item_id: &str) {
        if let Ok(Some(doc)) = self.current_document(item_id) {
            let ingested = crate::content::Ingested {
                excerpt: String::new(),
                lang: None,
                version: doc.version,
                document: doc.document,
            };
            self.index_item(item_id, &ingested);
        }
    }
}

impl Engine {
    /// Pairs this device with eddy-server and switches to server mode. Local
    /// data migrates: the FreshRSS account is given to the server if it has
    /// none, and every local change is uploaded and merged by HLC. The
    /// account also stays on this device, for refreshing while the server is
    /// offline.
    ///
    /// Nothing local changes until the server has accepted the device and
    /// the account. Once connected, a failed first sync does not undo the
    /// connection: it is retried like any other.
    ///
    /// Returns when the first sync is done. An app pairs with
    /// `pair_server` and lets `start_sync` do the rest in the background.
    pub async fn connect_server(
        &self,
        url: &str,
        pairing_code: &str,
        device_name: &str,
    ) -> Result<ServerStatus> {
        let mut status = self.pair_server(url, pairing_code, device_name).await?;
        self.first_sync().await;
        status.cursor = self
            .inner
            .store
            .meta_get(CURSOR_META)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(status)
    }

    /// Syncs with the server in the background, reporting through the
    /// progress and change listeners. Changes not uploaded when it fails
    /// are retried as usual.
    pub fn start_sync(&self) {
        let engine = self.clone();
        tokio::spawn(async move {
            engine.first_sync().await;
            engine.inner.notify_changed();
        });
    }

    async fn first_sync(&self) {
        let inner = &self.inner;
        // Let the server ingest FreshRSS first so uploaded read/star changes
        // land on items it already has.
        if let Some(client) = inner.hub_client()
            && let Err(e) = client.refresh().await
        {
            inner
                .platform
                .log(LogLevel::Warn, &format!("server refresh failed: {e}"));
        }
        if let Err(e) = self.refresh().await {
            inner
                .platform
                .log(LogLevel::Warn, &format!("first sync failed: {e}"));
            inner.schedule_push();
        }
    }

    /// The pairing half of `connect_server`: registers this device, settles
    /// the FreshRSS account and switches to server mode, without syncing.
    pub async fn pair_server(
        &self,
        url: &str,
        pairing_code: &str,
        device_name: &str,
    ) -> Result<ServerStatus> {
        let inner = &self.inner;
        let pairing = SyncClient::new(url, None)?;
        let paired = pairing
            .pair(&PairRequest {
                code: pairing_code.trim().to_string(),
                device_id: self.device_id(),
                device_name: device_name.to_string(),
            })
            .await?;
        let client = SyncClient::new(url, Some(paired.token.clone()))?;
        let mut info: ServerInfo = paired.info;
        info.check_protocol()?;

        let local_account = inner.direct.read().as_ref().map(|d| d.account.clone());
        match (&info.account, &local_account) {
            // Two different accounts would show every feed twice, with
            // reading state attached to articles the server does not have.
            (Some(remote), Some(local)) if server_account_id(remote) != local.id => {
                return Err(EngineError::Invalid(format!(
                    "服务端已经在用另一个 FreshRSS 账号（{} @ {}）。请先在这台设备上移除本机的 FreshRSS 账号，或让服务端改用同一个账号，再重新配对。",
                    remote.username, remote.base_url
                )));
            }
            (None, Some(account)) => {
                if let Some(password) = inner.platform.secret_get(&account_secret_key(&account.id))
                {
                    info = client
                        .set_account(&AccountRequest {
                            base_url: account.base_url.clone(),
                            username: account.username.clone(),
                            password,
                        })
                        .await?;
                }
            }
            _ => {}
        }
        if !inner.platform.secret_set(TOKEN_KEY, &paired.token) {
            return Err(EngineError::Invalid(
                "无法把服务端令牌保存到钥匙串。".into(),
            ));
        }
        inner.store.meta_set(URL_META, client.base_url())?;
        inner.adopt_server(&info)?;
        // FreshRSS pushes are now the server's job.
        let pending: Vec<i64> = inner
            .store
            .upstream_pending()?
            .iter()
            .map(|o| o.seq)
            .collect();
        inner.store.clear_upstream_pending(&pending)?;
        *inner.upstream.write() = Some(Arc::new(HubUpstream { client }));
        *inner.hub_down_since.lock() = None;
        let cursor = inner
            .store
            .meta_get(CURSOR_META)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(ServerStatus {
            url: url.to_string(),
            server_id: info.server_id,
            account: info.account,
            cursor,
            video_capable: info.video_capable,
            version: info.version,
        })
    }

    /// Leaves server mode. Local data stays, and with a FreshRSS account on
    /// this device it carries on in standalone mode: read/star changes the
    /// server never received go to FreshRSS instead.
    pub fn disconnect_server(&self) -> Result<()> {
        let inner = &self.inner;
        self.stop_live_sync();
        inner.platform.secret_delete(TOKEN_KEY);
        inner.store.meta_delete(URL_META)?;
        inner.store.meta_delete(CURSOR_META)?;
        inner.store.meta_delete(SERVER_ACCOUNT_META)?;
        *inner.hub_down_since.lock() = None;
        let direct = inner.direct.read().clone();
        if direct.is_some() {
            inner.store.requeue_unuploaded_for_upstream()?;
        }
        *inner.upstream.write() = direct.map(|d| d as Arc<dyn Upstream>);
        Ok(())
    }

    pub async fn server_status(&self) -> Result<Option<ServerStatus>> {
        let Some(client) = self.inner.hub_client() else {
            return Ok(None);
        };
        let info = client.info().await;
        self.inner
            .note_hub(&info.as_ref().map(|_| ()).map_err(|e| match e {
                SyncError::Network(m) => EngineError::Offline(m.clone()),
                other => EngineError::Network(other.to_string()),
            }));
        let info = info?;
        self.inner.adopt_server(&info)?;
        let cursor = self
            .inner
            .store
            .meta_get(CURSOR_META)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(Some(ServerStatus {
            url: client.base_url().to_string(),
            server_id: info.server_id,
            account: info.account,
            cursor,
            video_capable: info.video_capable,
            version: info.version,
        }))
    }

    /// Sets the FreshRSS account on the server (server mode).
    pub async fn configure_server_freshrss(
        &self,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        let client = self.inner.hub_client().ok_or(EngineError::NotConfigured)?;
        client
            .set_account(&AccountRequest {
                base_url: url.into(),
                username: username.into(),
                password: password.into(),
            })
            .await?;
        self.refresh().await.map(|_| ())
    }

    /// Searches the server's full history when connected, falling back to
    /// local search offline.
    pub async fn search_all(&self, query: &str, limit: u32) -> Result<Vec<SearchResult>> {
        // Right after the server failed to answer, search this device at
        // once instead of waiting for another timeout.
        let Some(client) = self.inner.reachable_hub_client() else {
            return self.search(query, limit);
        };
        let found = client.search(query, limit).await.map_err(EngineError::from);
        self.inner.note_hub(&found);
        match found {
            Ok(hits) => {
                let ids: Vec<String> = hits.iter().map(|h| h.item_id.clone()).collect();
                let rows = self.inner.store.items(&eddy_store::ItemQuery {
                    filter: eddy_store::ItemFilter::Ids(ids),
                    unread_only: false,
                    keep: Vec::new(),
                    limit: limit.max(1),
                    offset: 0,
                })?;
                Ok(hits
                    .into_iter()
                    .filter_map(|h| {
                        let item = rows.iter().find(|r| r.item.id == h.item_id)?.clone();
                        Some(SearchResult {
                            item,
                            snippet: h.snippet,
                            highlights: h.highlights,
                        })
                    })
                    .collect())
            }
            Err(_) => self.search(query, limit),
        }
    }

    /// While the app is open, listens for server changes and syncs as they
    /// arrive. Reconnects with backoff until stopped. Starting it again
    /// while it runs does nothing.
    pub fn start_live_sync(&self) {
        let mut live = self.inner.live.lock();
        if live.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let engine = self.clone();
        *live = Some(tokio::spawn(async move {
            let mut backoff = Duration::from_secs(2);
            loop {
                let Some(client) = engine.inner.hub_client() else {
                    break;
                };
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<i64>();
                // Dropped with this task, so stopping closes the connection.
                let listener = AbortOnDrop(tokio::spawn(async move {
                    client
                        .listen(move |cursor| {
                            let _ = tx.send(cursor);
                        })
                        .await
                }));
                let mut greeted = false;
                while let Some(cursor) = rx.recv().await {
                    backoff = Duration::from_secs(2);
                    if cursor < 0 {
                        engine.inner.notify_video_listener();
                        continue;
                    }
                    let local: i64 = engine
                        .inner
                        .store
                        .meta_get(CURSOR_META)
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    // The first message means the server is reachable
                    // (again): changes made meanwhile go up now.
                    let pending = !std::mem::replace(&mut greeted, true)
                        && engine
                            .inner
                            .store
                            .upload_counts()
                            .is_ok_and(|(pending, _)| pending > 0);
                    *engine.inner.hub_down_since.lock() = None;
                    if cursor > local || pending {
                        if let Err(e) = engine.refresh().await {
                            engine
                                .inner
                                .platform
                                .log(LogLevel::Warn, &format!("live sync failed: {e}"));
                        }
                        engine.inner.notify_video_listener();
                    }
                    engine.inner.notify_changed();
                }
                drop(listener);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }));
    }

    pub fn stop_live_sync(&self) {
        if let Some(task) = self.inner.live.lock().take() {
            task.abort();
        }
    }

    /// Ensures blobs of the item's current version are local, downloading
    /// them from the server when needed.
    pub(crate) async fn fetch_missing_blobs(&self, item_id: &str) -> Result<bool> {
        let inner = &self.inner;
        let Some(row) = inner.store.item(item_id)? else {
            return Ok(false);
        };
        let Some(hash) = row.item.current_version else {
            return Ok(false);
        };
        let Some(version) = inner.store.content_version(&hash)? else {
            return Ok(false);
        };
        let upstream = inner.upstream.read().clone();
        let Some(upstream) = upstream else {
            return Ok(false);
        };
        let mut fetched = false;
        let doc_missing = version
            .doc_blob
            .as_ref()
            .is_none_or(|k| !inner.store.has_blob(k).unwrap_or(false));
        let raw_missing = !inner.store.has_blob(&version.raw_blob)?;
        let keys: Vec<&String> = if doc_missing {
            [
                version.doc_blob.as_ref(),
                raw_missing.then_some(&version.raw_blob),
            ]
            .into_iter()
            .flatten()
            .collect()
        } else {
            Vec::new()
        };
        for key in keys {
            if let Some((bytes, mime)) = upstream.fetch_blob(key).await? {
                inner.store.put_blob_keyed(
                    key,
                    &bytes,
                    &mime,
                    row.item.kind != eddy_model::ItemKind::Feed && key == &version.raw_blob,
                )?;
                fetched = true;
            }
        }
        if fetched {
            inner.index_current(item_id);
            inner.commit_index();
        }
        Ok(fetched)
    }
}

/// Clock check used by servers: rejects ops stamped implausibly far in the
/// future, which would win every later merge.
pub fn plausible_hlc(hlc: &Hlc) -> bool {
    let now = eddy_model::now_ms().max(0) as u64;
    hlc.wall_ms <= now + 24 * 3600 * 1000
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

/// Clears the published sync progress however the sync ends.
struct ProgressGuard<'a>(&'a Inner);

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        self.0.set_sync_progress(None);
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// ID of the account a server reports, as this device would compute it for
/// the same FreshRSS address and user.
fn server_account_id(account: &eddy_sync::AccountInfo) -> String {
    let base = eddy_source_greader::normalize_base(&account.base_url)
        .unwrap_or_else(|_| account.base_url.clone());
    ids::account_id("greader", &base, account.username.trim())
}
