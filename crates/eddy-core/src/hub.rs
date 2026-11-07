use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_trait::async_trait;
use eddy_model::{ContentVersion, Entity, Hlc, Op, fields, ids};
use eddy_sync::{AccountRequest, PairRequest, ServerInfo, SyncClient, SyncError};

use crate::engine::{Engine, Inner, RefreshSummary};
use crate::platform::account_secret_key;
use crate::search::SearchResult;
use crate::upstream::Upstream;
use crate::{EngineError, LogLevel, Result};

const TOKEN_KEY: &str = "server:token";
const URL_META: &str = "server_url";
const CURSOR_META: &str = "sync_cursor";
const PUSH_BATCH: u32 = 500;
const PULL_BATCH: u32 = 1000;
const PREFETCH_CAP: usize = 200;

impl From<SyncError> for EngineError {
    fn from(e: SyncError) -> Self {
        EngineError::Network(e.to_string())
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

    /// Synced AI results travel as artifact ops generated at upload time.
    async fn upload_artifacts(&self, inner: &Inner) -> Result<()> {
        let artifacts = inner.store.unuploaded_ai_artifacts()?;
        if artifacts.is_empty() {
            return Ok(());
        }
        let mut ops = Vec::new();
        let mut keys = Vec::new();
        for (key, item_id, kind, content_hash, lang, model, payload) in artifacts {
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
        Ok(())
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
            if local.iter().any(|(n, _)| n == name)
                || !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                continue;
            }
            std::fs::create_dir_all(&dir).map_err(|e| EngineError::Invalid(e.to_string()))?;
            std::fs::write(dir.join(format!("{name}.toml")), &entry.value)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
            changed = true;
        }
        if changed {
            inner.rebuild_fetcher()?;
        }
        Ok(())
    }

    /// Downloads document blobs for content that should be readable offline:
    /// unread, starred, saved or in progress.
    async fn prefetch(&self, inner: &Inner, versions: Vec<String>) {
        for hash in versions.into_iter().take(PREFETCH_CAP) {
            let Ok(Some(version)) = inner.store.content_version(&hash) else {
                continue;
            };
            let Ok(Some(row)) = inner.store.item(&version.item_id) else {
                continue;
            };
            let wanted = !row.item.read
                || row.item.starred
                || row.item.kind != eddy_model::ItemKind::Feed
                || (row.item.progress > 0.0 && !row.item.finished);
            if !wanted || row.item.current_version.as_deref() != Some(hash.as_str()) {
                continue;
            }
            if let Some(key) = &version.doc_blob {
                if inner.store.has_blob(key).unwrap_or(true) {
                    continue;
                }
                if let Ok(Some((bytes, mime))) = self.client.get_blob(key).await
                    && inner
                        .store
                        .put_blob_keyed(key, &bytes, &mime, false)
                        .is_ok()
                {
                    inner.index_current(&version.item_id);
                }
            }
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
        loop {
            let pending = inner.store.unuploaded_ops(PUSH_BATCH)?;
            if pending.is_empty() {
                break;
            }
            self.upload_blobs(inner, &pending).await?;
            let seqs: Vec<i64> = pending.iter().map(|o| o.seq).collect();
            self.client
                .push(pending.into_iter().map(|o| o.op).collect())
                .await?;
            inner.store.mark_uploaded(&seqs)?;
        }
        self.upload_artifacts(inner).await
    }

    async fn sync(&self, inner: &Inner) -> Result<RefreshSummary> {
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
        let mut state_changes = 0;
        loop {
            let page = self.client.pull(cursor, PULL_BATCH).await?;
            for op in &page.ops {
                inner.clock.observe(&op.hlc);
                let applied = inner.store.apply_remote(op)?;
                match (op.entity, op.field.as_str()) {
                    // Items the server ingested already read (backfilled
                    // history included) are not new to this device.
                    (Entity::Item, fields::CREATE) if applied && op.value["read"] != true => {
                        new_item_ids.push(op.entity_id.clone())
                    }
                    (Entity::Item, fields::VERSION) if applied => {
                        versions.push(op.value.as_str().unwrap_or_default().to_string())
                    }
                    (Entity::Item, fields::READ | fields::STARRED) if applied => state_changes += 1,
                    _ => {}
                }
            }
            cursor = page.cursor;
            inner.store.meta_set(CURSOR_META, &cursor.to_string())?;
            if !page.more {
                break;
            }
        }
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

    fn hub_client(&self) -> Option<SyncClient> {
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
    /// data migrates: the FreshRSS account moves to the server if it has
    /// none, and every local change is uploaded and merged by HLC.
    pub async fn connect_server(
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

        let local_account = inner.direct.read().as_ref().map(|d| d.account.clone());
        if info.account.is_none()
            && let Some(account) = &local_account
            && let Some(password) = inner.platform.secret_get(&account_secret_key(&account.id))
        {
            info = client
                .set_account(&AccountRequest {
                    base_url: account.base_url.clone(),
                    username: account.username.clone(),
                    password,
                })
                .await?;
        }
        // Let the server ingest FreshRSS first so uploaded read/star changes
        // land on items it already has.
        if let Err(e) = client.refresh().await {
            inner
                .platform
                .log(LogLevel::Warn, &format!("server refresh failed: {e}"));
        }

        if !inner.platform.secret_set(TOKEN_KEY, &paired.token) {
            return Err(EngineError::Invalid(
                "无法把服务端令牌保存到钥匙串。".into(),
            ));
        }
        inner.store.meta_set(URL_META, client.base_url())?;
        for account in inner.store.accounts()? {
            inner
                .platform
                .secret_delete(&account_secret_key(&account.id));
            inner.store.delete_account(&account.id)?;
        }
        *inner.direct.write() = None;
        // FreshRSS pushes are now the server's job.
        let pending: Vec<i64> = inner
            .store
            .upstream_pending()?
            .iter()
            .map(|o| o.seq)
            .collect();
        inner.store.clear_upstream_pending(&pending)?;
        *inner.upstream.write() = Some(Arc::new(HubUpstream { client }));

        self.refresh().await?;
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
        })
    }

    /// Leaves server mode. Local data stays; FreshRSS can be configured
    /// again for standalone use.
    pub fn disconnect_server(&self) -> Result<()> {
        self.stop_live_sync();
        self.inner.platform.secret_delete(TOKEN_KEY);
        self.inner.store.meta_delete(URL_META)?;
        self.inner.store.meta_delete(CURSOR_META)?;
        *self.inner.upstream.write() = None;
        Ok(())
    }

    pub async fn server_status(&self) -> Result<Option<ServerStatus>> {
        let Some(client) = self.inner.hub_client() else {
            return Ok(None);
        };
        let info = client.info().await?;
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
        let Some(client) = self.inner.hub_client() else {
            return self.search(query, limit);
        };
        match client.search(query, limit).await {
            Ok(hits) => {
                let ids: Vec<String> = hits.iter().map(|h| h.item_id.clone()).collect();
                let rows = self.inner.store.items(&eddy_store::ItemQuery {
                    filter: eddy_store::ItemFilter::Ids(ids),
                    unread_only: false,
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
    /// arrive. Reconnects with backoff until stopped.
    pub fn start_live_sync(&self) {
        let inner = &self.inner;
        if inner.live.swap(true, Ordering::SeqCst) {
            return;
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(2);
            while engine.inner.live.load(Ordering::SeqCst) {
                let Some(client) = engine.inner.hub_client() else {
                    break;
                };
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<i64>();
                let listener = tokio::spawn(async move {
                    client
                        .listen(move |cursor| {
                            let _ = tx.send(cursor);
                        })
                        .await
                });
                while let Some(cursor) = rx.recv().await {
                    backoff = Duration::from_secs(2);
                    let local: i64 = engine
                        .inner
                        .store
                        .meta_get(CURSOR_META)
                        .ok()
                        .flatten()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    if cursor > local
                        && engine.inner.live.load(Ordering::SeqCst)
                        && let Err(e) = engine.refresh().await
                    {
                        engine
                            .inner
                            .platform
                            .log(LogLevel::Warn, &format!("live sync failed: {e}"));
                    }
                }
                let _ = listener.await;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
            engine.inner.live.store(false, Ordering::SeqCst);
        });
    }

    pub fn stop_live_sync(&self) {
        self.inner.live.store(false, Ordering::SeqCst);
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
