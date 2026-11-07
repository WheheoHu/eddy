//! SQLite-backed local store: items, content versions, the operation log with
//! last-writer-wins merging, and a content-addressed blob directory.

mod blobs;
mod items;
mod ops;
mod schema;

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

pub use items::{ItemFilter, ItemQuery, ItemRow};
pub use ops::{OpOrigin, StoredOp};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("data error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Mutex<Connection>,
    blobs_dir: PathBuf,
}

impl Store {
    /// Opens (creating if needed) the store in `dir`: `eddy.sqlite3` plus a
    /// `blobs/` directory.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let blobs_dir = dir.join("blobs");
        std::fs::create_dir_all(&blobs_dir)?;
        let mut conn = Connection::open(dir.join("eddy.sqlite3"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn)?;
        Ok(Store { conn: Mutex::new(conn), blobs_dir })
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock();
        f(&mut conn)
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| Ok(c.query_row("SELECT value FROM meta WHERE key=?", [key], |r| r.get(0)).optional()?))
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT INTO meta(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
            Ok(())
        })
    }

    pub fn meta_delete(&self, key: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM meta WHERE key=?", [key])?;
            Ok(())
        })
    }

    /// Stable identifier of this installation, also the HLC node name.
    pub fn device_id(&self) -> Result<String> {
        if let Some(id) = self.meta_get("device_id")? {
            return Ok(id);
        }
        let id = eddy_model::ids::new_local_id();
        self.meta_set("device_id", &id)?;
        Ok(id)
    }

    /// Settings are synced key/value pairs written through ops.
    pub fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let raw: Option<String> =
            self.with(|c| Ok(c.query_row("SELECT value FROM settings WHERE key=?", [key], |r| r.get(0)).optional()?))?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn upsert_account(&self, a: &eddy_model::Account) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO accounts(id,kind,base_url,username,created_at) VALUES(?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET base_url=excluded.base_url, username=excluded.username",
                params![a.id, a.kind, a.base_url, a.username, a.created_at],
            )?;
            Ok(())
        })
    }

    pub fn accounts(&self) -> Result<Vec<eddy_model::Account>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT id,kind,base_url,username,created_at FROM accounts ORDER BY created_at")?;
            let rows = stmt.query_map([], |r| {
                Ok(eddy_model::Account {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    base_url: r.get(2)?,
                    username: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM accounts WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn ai_usage_today(&self, day: &str) -> Result<u32> {
        self.with(|c| {
            Ok(c.query_row("SELECT count FROM ai_usage WHERE day=?", [day], |r| r.get::<_, u32>(0)).optional()?.unwrap_or(0))
        })
    }

    pub fn add_ai_usage(&self, day: &str, n: u32) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO ai_usage(day,count) VALUES(?,?) ON CONFLICT(day) DO UPDATE SET count=count+excluded.count",
                params![day, n],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
