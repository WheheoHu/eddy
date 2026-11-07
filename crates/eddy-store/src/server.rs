//! Tables only a sync server uses: paired devices, one-time pairing codes and
//! private configuration (rule packs) distributed to devices.

use eddy_model::now_ms;
use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen: Option<i64>,
}

impl Store {
    pub fn add_pairing_code(&self, code_hash: &str, expires_at: i64) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM pairing_codes WHERE expires_at<?", [now_ms()])?;
            c.execute(
                "INSERT OR REPLACE INTO pairing_codes(code_hash,expires_at) VALUES(?,?)",
                params![code_hash, expires_at],
            )?;
            Ok(())
        })
    }

    /// Consumes a pairing code; true when it existed and had not expired.
    pub fn consume_pairing_code(&self, code_hash: &str) -> Result<bool> {
        self.with(|c| {
            let n = c.execute(
                "DELETE FROM pairing_codes WHERE code_hash=? AND expires_at>=?",
                params![code_hash, now_ms()],
            )?;
            Ok(n > 0)
        })
    }

    pub fn add_device(&self, id: &str, name: &str, token_hash: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO devices(id,name,token_hash,created_at) VALUES(?,?,?,?)",
                params![id, name, token_hash, now_ms()],
            )?;
            Ok(())
        })
    }

    /// The device owning a token hash, updating its last-seen time.
    pub fn device_by_token(&self, token_hash: &str) -> Result<Option<String>> {
        self.with(|c| {
            let id: Option<String> = c
                .query_row(
                    "SELECT id FROM devices WHERE token_hash=?",
                    [token_hash],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = &id {
                c.execute(
                    "UPDATE devices SET last_seen=? WHERE id=?",
                    params![now_ms(), id],
                )?;
            }
            Ok(id)
        })
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT id,name,created_at,last_seen FROM devices ORDER BY created_at")?;
            let rows = stmt.query_map([], |r| {
                Ok(Device {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    last_seen: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn remove_device(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM devices WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn config_set(&self, key: &str, value: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO server_config(key,value,updated_at) VALUES(?,?,?)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
                params![key, value, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn config_delete(&self, key: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM server_config WHERE key=?", [key])?;
            Ok(())
        })
    }

    /// All config entries: (key, value, updated_at).
    pub fn config_entries(&self) -> Result<Vec<(String, String, i64)>> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT key,value,updated_at FROM server_config ORDER BY key")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
}
