use std::path::PathBuf;

use eddy_model::{ids::sha256_hex, now_ms};
use rusqlite::{OptionalExtension, params};

use crate::{Result, Store, StoreError};

/// Blobs of deleted items that no live item uses. Libraries hold hundreds of
/// thousands of blobs, so this starts from the deleted items (`CROSS JOIN`
/// keeps that loop order in SQLite) and checks each column through its own
/// index; an `OR` across the two columns scans them.
pub(crate) const ORPHAN_BLOBS: &str = "
    WITH candidates(key) AS (
        SELECT cv.raw_blob FROM items i CROSS JOIN content_versions cv ON cv.item_id=i.id
         WHERE i.deleted=1
        UNION
        SELECT cv.doc_blob FROM items i CROSS JOIN content_versions cv ON cv.item_id=i.id
         WHERE i.deleted=1 AND cv.doc_blob IS NOT NULL)
    SELECT b.key, b.size FROM candidates c CROSS JOIN blobs b ON b.key=c.key
     WHERE NOT EXISTS (
            SELECT 1 FROM content_versions cv JOIN items i ON i.id=cv.item_id
             WHERE cv.raw_blob=b.key AND i.deleted=0)
       AND NOT EXISTS (
            SELECT 1 FROM content_versions cv JOIN items i ON i.id=cv.item_id
             WHERE cv.doc_blob=b.key AND i.deleted=0)";

impl Store {
    fn blob_path(&self, key: &str) -> Result<PathBuf> {
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(StoreError::Invalid(format!("invalid blob key {key:?}")));
        }
        let shard = &key[key.len().saturating_sub(2)..];
        Ok(self.blobs_dir.join(shard).join(key))
    }

    /// Stores `bytes` under their SHA-256 and returns the key.
    pub fn put_blob(&self, bytes: &[u8], mime: &str, pinned: bool) -> Result<String> {
        let key = sha256_hex(bytes);
        self.put_blob_keyed(&key, bytes, mime, pinned)?;
        Ok(key)
    }

    /// Stores `bytes` under a caller-chosen key (e.g. a hashed image URL).
    pub fn put_blob_keyed(&self, key: &str, bytes: &[u8], mime: &str, pinned: bool) -> Result<()> {
        let path = self.blob_path(key)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO blobs(key,size,mime,pinned,last_access) VALUES(?,?,?,?,?)
                 ON CONFLICT(key) DO UPDATE SET size=excluded.size, mime=excluded.mime,
                   pinned=MAX(pinned, excluded.pinned), last_access=excluded.last_access",
                params![key, bytes.len() as i64, mime, pinned as i64, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn get_blob(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.blob_path(key)?;
        match std::fs::read(&path) {
            Ok(bytes) => {
                self.with(|c| {
                    c.execute(
                        "UPDATE blobs SET last_access=? WHERE key=?",
                        params![now_ms(), key],
                    )?;
                    Ok(())
                })?;
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn blob_mime(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| {
            Ok(
                c.query_row("SELECT mime FROM blobs WHERE key=?", [key], |r| r.get(0))
                    .optional()?
                    .flatten(),
            )
        })
    }

    pub fn has_blob(&self, key: &str) -> Result<bool> {
        Ok(self.blob_path(key)?.exists())
    }

    /// The file holding a blob, for readers that seek (archives) or open
    /// files themselves (PDF views), when it is stored locally.
    pub fn blob_file(&self, key: &str) -> Result<Option<PathBuf>> {
        let path = self.blob_path(key)?;
        Ok(path.exists().then_some(path))
    }

    pub fn pin_blob(&self, key: &str, pinned: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE blobs SET pinned=? WHERE key=?",
                params![pinned as i64, key],
            )?;
            Ok(())
        })
    }

    pub fn blob_usage(&self) -> Result<u64> {
        self.with(|c| {
            Ok(
                c.query_row("SELECT COALESCE(SUM(size),0) FROM blobs", [], |r| {
                    r.get::<_, i64>(0)
                })? as u64,
            )
        })
    }

    /// Deletes the content blobs that only deleted items use, original files
    /// of imported books and PDFs included. Returns bytes freed.
    pub fn purge_orphan_blobs(&self) -> Result<u64> {
        let orphans: Vec<(String, u64)> = self.with(|c| {
            let mut stmt = c.prepare(ORPHAN_BLOBS)?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        let mut freed = 0;
        for (key, size) in orphans {
            self.remove_blob(&key)?;
            freed += size;
        }
        Ok(freed)
    }

    fn remove_blob(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.blob_path(key)?) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.with(|c| {
            c.execute("DELETE FROM blobs WHERE key=?", [key])?;
            c.execute(
                "UPDATE content_versions SET doc_blob=NULL, doc_schema=NULL WHERE doc_blob=?",
                [key],
            )?;
            Ok(())
        })
    }

    /// Deletes least recently used blobs until usage is at most `limit` bytes.
    /// Pinned blobs and content of items that are unread, starred, in progress
    /// or not plain subscription articles are kept. Returns bytes freed.
    pub fn evict(&self, limit: u64) -> Result<u64> {
        let mut usage = self.blob_usage()?;
        if usage <= limit {
            return Ok(0);
        }
        let candidates: Vec<(String, u64)> = self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT b.key, b.size FROM blobs b WHERE b.pinned=0 AND NOT EXISTS (
                    SELECT 1 FROM content_versions cv JOIN items i ON i.id=cv.item_id
                    WHERE cv.raw_blob=b.key
                      AND (i.starred=1 OR i.read=0 OR (i.progress>0 AND i.finished=0) OR i.kind!='feed'))
                 AND NOT EXISTS (
                    SELECT 1 FROM content_versions cv JOIN items i ON i.id=cv.item_id
                    WHERE cv.doc_blob=b.key
                      AND (i.starred=1 OR i.read=0 OR (i.progress>0 AND i.finished=0) OR i.kind!='feed'))
                 ORDER BY b.last_access ASC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        let mut freed = 0;
        for (key, size) in candidates {
            if usage <= limit {
                break;
            }
            self.remove_blob(&key)?;
            usage = usage.saturating_sub(size);
            freed += size;
        }
        Ok(freed)
    }
}
