//! Engine operations used by eddy-server. The server runs the same engine in
//! server role: its op log is the change feed clients pull from.

use eddy_model::{Entity, Op, fields, ids::sha256_hex};

use crate::engine::Engine;
use crate::hub::plausible_hlc;
use crate::{EngineError, Result};

impl Engine {
    /// The FreshRSS account the server manages, if configured.
    pub fn server_account(&self) -> Option<eddy_model::Account> {
        self.inner.direct.read().as_ref().map(|d| d.account.clone())
    }

    /// Applies ops uploaded by `device`. Read/star changes on FreshRSS items
    /// are queued for the account and pushed in the background. Returns how
    /// many ops won their merge and the new cursor.
    pub fn server_apply_ops(&self, device: &str, ops: &[Op]) -> Result<(u32, i64)> {
        let inner = &self.inner;
        let mut applied = 0;
        let mut needs_push = false;
        for op in ops {
            if !plausible_hlc(&op.hlc) {
                return Err(EngineError::Invalid(
                    "操作的时间戳超前太多，请检查设备时间。".into(),
                ));
            }
            inner.clock.observe(&op.hlc);
            let upstream_pending = op.entity == Entity::Item
                && (op.field == fields::READ || op.field == fields::STARRED)
                && inner
                    .store
                    .item(&op.entity_id)?
                    .is_some_and(|r| r.item.upstream_id.is_some());
            if inner
                .store
                .apply_remote_from(op, Some(device), upstream_pending)?
            {
                applied += 1;
                needs_push |= upstream_pending;
                if op.entity == Entity::Item && op.field == fields::VERSION {
                    inner.index_current(&op.entity_id);
                }
            }
        }
        if needs_push {
            inner.push_in_background();
        }
        inner.commit_index();
        Ok((applied, inner.store.max_op_seq()?))
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
