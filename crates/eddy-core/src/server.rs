//! Engine operations used by eddy-server. The server runs the same engine in
//! server role: its op log is the change feed clients pull from.

use eddy_model::{Entity, Op, fields, ids::sha256_hex};

use crate::engine::Engine;
use crate::hub::plausible_hlc;
use crate::{EngineError, Result};

/// Outcome of a device's push.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppliedOps {
    /// Ops that won their merge.
    pub applied: u32,
    /// The server's cursor afterwards.
    pub cursor: i64,
    /// Ops refused for good: (op ID, reason).
    pub rejected: Vec<(String, String)>,
}

impl Engine {
    /// The FreshRSS account the server manages, if configured.
    pub fn server_account(&self) -> Option<eddy_model::Account> {
        self.inner.direct.read().as_ref().map(|d| d.account.clone())
    }

    /// Applies ops uploaded by `device`. Read/star changes on FreshRSS items
    /// are queued for the account and pushed in the background.
    ///
    /// An op the server cannot use (it does not parse, its value is wrong,
    /// its clock is far ahead) is refused on its own: the rest of the batch
    /// still applies, and the device stops sending that one.
    pub fn server_apply_ops(&self, device: &str, ops: &[serde_json::Value]) -> Result<AppliedOps> {
        let inner = &self.inner;
        let mut applied = 0;
        let mut needs_push = false;
        let mut rejected = Vec::new();
        for raw in ops {
            let op_id = raw["op_id"].as_str().unwrap_or_default().to_string();
            let op: Op = match serde_json::from_value(raw.clone()) {
                Ok(op) => op,
                Err(_) => {
                    rejected.push((op_id, "这个版本的服务端不认识这种改动。".to_string()));
                    continue;
                }
            };
            if !plausible_hlc(&op.hlc) {
                rejected.push((op_id, "操作的时间戳超前太多，请检查设备时间。".to_string()));
                continue;
            }
            inner.clock.observe(&op.hlc);
            let upstream_pending = op.entity == Entity::Item
                && (op.field == fields::READ || op.field == fields::STARRED)
                && inner
                    .store
                    .item(&op.entity_id)?
                    .is_some_and(|r| r.item.upstream_id.is_some());
            match inner
                .store
                .apply_remote_from(&op, Some(device), upstream_pending)
            {
                Ok(true) => {
                    applied += 1;
                    needs_push |= upstream_pending;
                    if op.entity == Entity::Item && op.field == fields::VERSION {
                        inner.index_current(&op.entity_id);
                    }
                }
                Ok(false) => {}
                // The op itself is unusable; anything else (disk, database)
                // is the server's problem and fails the request.
                Err(e @ (eddy_store::StoreError::Invalid(_) | eddy_store::StoreError::Json(_))) => {
                    rejected.push((op_id, e.to_string()))
                }
                Err(e) => return Err(e.into()),
            }
        }
        if needs_push {
            inner.push_in_background();
        }
        inner.commit_index();
        Ok(AppliedOps {
            applied,
            cursor: inner.store.max_op_seq()?,
            rejected,
        })
    }

    /// Whether the database answers, for health checks.
    pub fn server_healthy(&self) -> bool {
        self.inner.store.max_op_seq().is_ok()
    }

    /// Ops after `since`: (ops, cursor, more).
    pub fn server_ops_since(&self, since: i64, limit: u32) -> Result<(Vec<Op>, i64, bool)> {
        let limit = limit.clamp(1, 5000);
        let page = self.inner.store.ops_since(since, limit)?;
        let more = page.len() as u32 == limit;
        let cursor = page.last().map(|(o, _)| o.seq).unwrap_or(since.max(0));
        Ok((page.into_iter().map(|(o, _)| o.op).collect(), cursor, more))
    }

    pub fn server_cursor(&self) -> Result<i64> {
        Ok(self.inner.store.max_op_seq()?)
    }

    pub fn server_blob(&self, key: &str) -> Result<Option<(Vec<u8>, String)>> {
        let Some(bytes) = self.inner.store.get_blob(key)? else {
            return Ok(None);
        };
        let mime = self
            .inner
            .store
            .blob_mime(key)?
            .unwrap_or_else(|| "application/octet-stream".into());
        Ok(Some((bytes, mime)))
    }

    pub fn server_has_blob(&self, key: &str) -> Result<bool> {
        Ok(self.inner.store.has_blob(key)?)
    }

    /// Stores a blob uploaded by a client. Keys are content hashes, verified
    /// here so a device cannot overwrite content under another hash.
    pub fn server_put_blob(&self, key: &str, bytes: &[u8], mime: &str) -> Result<()> {
        if sha256_hex(bytes) != key {
            return Err(EngineError::Invalid("内容与哈希不符。".into()));
        }
        // Uploaded blobs are user content (saved pages); keep them.
        self.inner.store.put_blob_keyed(key, bytes, mime, true)?;
        Ok(())
    }
}
