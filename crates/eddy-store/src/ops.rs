use eddy_model::{Entity, Hlc, Item, Locator, Op, fields};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

use crate::{Result, Store, StoreError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpOrigin {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredOp {
    pub seq: i64,
    pub op: Op,
}

impl Store {
    /// Records a local change in the log and applies it. `upstream_pending`
    /// marks changes that the direct upstream (FreshRSS) still has to receive.
    pub fn record_local(&self, op: &Op, upstream_pending: bool) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            apply(&tx, op, OpOrigin::Local, upstream_pending, None)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Applies an op received from another device. Returns whether it won the
    /// last-writer-wins comparison for its field.
    pub fn apply_remote(&self, op: &Op) -> Result<bool> {
        self.apply_remote_from(op, None, false)
    }

    /// Applies an op from `device`; on a server, `upstream_pending` queues it
    /// for the FreshRSS account the server manages.
    pub fn apply_remote_from(
        &self,
        op: &Op,
        device: Option<&str>,
        upstream_pending: bool,
    ) -> Result<bool> {
        self.with(|c| {
            let tx = c.transaction()?;
            let applied = apply(&tx, op, OpOrigin::Remote, upstream_pending, device)?;
            tx.commit()?;
            Ok(applied)
        })
    }

    /// The change feed: ops after `seq`, with the device each came from.
    pub fn ops_since(&self, seq: i64, limit: u32) -> Result<Vec<(StoredOp, Option<String>)>> {
        let devices: std::collections::HashMap<i64, Option<String>> = self.with(|c| {
            let mut stmt =
                c.prepare("SELECT seq, device FROM ops WHERE seq>? ORDER BY seq LIMIT ?")?;
            let rows = stmt.query_map(params![seq, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })?;
        let ops = self.query_ops(&format!("WHERE seq>{seq} ORDER BY seq"), limit)?;
        Ok(ops
            .into_iter()
            .map(|o| {
                let device = devices.get(&o.seq).cloned().flatten();
                (o, device)
            })
            .collect())
    }

    pub fn max_op_seq(&self) -> Result<i64> {
        self.with(|c| Ok(c.query_row("SELECT COALESCE(MAX(seq),0) FROM ops", [], |r| r.get(0))?))
    }

    pub fn upstream_pending(&self) -> Result<Vec<StoredOp>> {
        self.query_ops("WHERE upstream_pending=1 ORDER BY seq", u32::MAX)
    }

    pub fn clear_upstream_pending(&self, seqs: &[i64]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            for seq in seqs {
                tx.execute("UPDATE ops SET upstream_pending=0 WHERE seq=?", [seq])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Local ops not yet sent to a sync server, oldest first.
    pub fn unuploaded_ops(&self, limit: u32) -> Result<Vec<StoredOp>> {
        self.query_ops("WHERE uploaded=0 ORDER BY seq", limit)
    }

    pub fn mark_uploaded(&self, seqs: &[i64]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            for seq in seqs {
                tx.execute("UPDATE ops SET uploaded=1 WHERE seq=?", [seq])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Current winning timestamp for a field, if any op touched it.
    pub fn field_clock(&self, entity: Entity, entity_id: &str, field: &str) -> Result<Option<Hlc>> {
        let raw: Option<String> = self.with(|c| {
            Ok(c.query_row(
                "SELECT hlc FROM field_clock WHERE entity=? AND entity_id=? AND field=?",
                params![entity.as_str(), entity_id, field],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        Ok(raw.and_then(|s| s.parse().ok()))
    }

    fn query_ops(&self, tail: &str, limit: u32) -> Result<Vec<StoredOp>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT seq, op_id, hlc, entity, entity_id, field, value FROM ops {tail} LIMIT {limit}"
            ))?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (seq, op_id, hlc, entity, entity_id, field, value) = row?;
                let (Ok(hlc), Some(entity)) = (hlc.parse(), Entity::parse(&entity)) else { continue };
                out.push(StoredOp { seq, op: Op { op_id, hlc, entity, entity_id, field, value: serde_json::from_str(&value)? } });
            }
            Ok(out)
        })
    }
}

fn apply(
    tx: &Transaction,
    op: &Op,
    origin: OpOrigin,
    upstream_pending: bool,
    device: Option<&str>,
) -> Result<bool> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO ops(op_id,hlc,entity,entity_id,field,value,origin,uploaded,upstream_pending,device)
         VALUES(?,?,?,?,?,?,?,?,?,?)",
        params![
            op.op_id,
            op.hlc.to_string(),
            op.entity.as_str(),
            op.entity_id,
            op.field,
            serde_json::to_string(&op.value)?,
            if origin == OpOrigin::Local { "local" } else { "remote" },
            origin == OpOrigin::Remote,
            upstream_pending,
            device,
        ],
    )?;
    if inserted == 0 {
        return Ok(false);
    }
    let current: Option<String> = tx
        .query_row(
            "SELECT hlc FROM field_clock WHERE entity=? AND entity_id=? AND field=?",
            params![op.entity.as_str(), op.entity_id, op.field],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(current) = current.and_then(|s| s.parse::<Hlc>().ok())
        && current >= op.hlc
    {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO field_clock(entity,entity_id,field,hlc) VALUES(?,?,?,?)
         ON CONFLICT(entity,entity_id,field) DO UPDATE SET hlc=excluded.hlc",
        params![
            op.entity.as_str(),
            op.entity_id,
            op.field,
            op.hlc.to_string()
        ],
    )?;
    effect(tx, op)?;
    Ok(true)
}

fn bool_value(v: &Value) -> Result<bool> {
    v.as_bool()
        .ok_or_else(|| StoreError::Invalid("expected boolean".into()))
}

fn str_value(v: &Value) -> Result<&str> {
    v.as_str()
        .ok_or_else(|| StoreError::Invalid("expected string".into()))
}

fn effect(tx: &Transaction, op: &Op) -> Result<()> {
    let id = op.entity_id.as_str();
    let v = &op.value;
    match (op.entity, op.field.as_str()) {
        (Entity::Item, fields::CREATE) => {
            let item: Item = serde_json::from_value(v.clone())?;
            crate::items::insert_item(tx, &item)?;
        }
        (Entity::Item, fields::READ) => {
            tx.execute(
                "UPDATE items SET read=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::STARRED) => {
            tx.execute(
                "UPDATE items SET starred=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::FINISHED) => {
            tx.execute(
                "UPDATE items SET finished=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::DELETED) => {
            tx.execute(
                "UPDATE items SET deleted=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::TITLE) => {
            tx.execute(
                "UPDATE items SET title=? WHERE id=?",
                params![str_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::VERSION) => {
            tx.execute(
                "UPDATE items SET current_version=? WHERE id=?",
                params![str_value(v)?, id],
            )?;
        }
        (Entity::Item, fields::SERIES) => {
            let series = v.get("series_id").and_then(Value::as_str);
            let index = v.get("chapter_index").and_then(Value::as_i64);
            tx.execute(
                "UPDATE items SET series_id=?, chapter_index=? WHERE id=?",
                params![series, index, id],
            )?;
        }
        (Entity::Progress, fields::LOCATOR) => {
            let locator: Locator = serde_json::from_value(v.clone())?;
            tx.execute(
                "INSERT INTO progress(item_id,locator,progression,updated_at,device) VALUES(?,?,?,?,?)
                 ON CONFLICT(item_id) DO UPDATE SET locator=excluded.locator, progression=excluded.progression,
                   updated_at=excluded.updated_at, device=excluded.device",
                params![id, serde_json::to_string(&locator)?, locator.progression, op.hlc.wall_ms as i64, op.hlc.node],
            )?;
            tx.execute(
                "UPDATE items SET progress=?, last_read_at=? WHERE id=?",
                params![locator.progression, op.hlc.wall_ms as i64, id],
            )?;
        }
        (Entity::Annotation, fields::CREATE) => {
            tx.execute(
                "INSERT OR IGNORE INTO annotations(id,item_id,locator,kind,color,note,created_at) VALUES(?,?,?,?,?,?,?)",
                params![
                    id,
                    v.get("item_id").and_then(Value::as_str),
                    v.get("locator").map(|l| l.to_string()),
                    v.get("kind").and_then(Value::as_str).unwrap_or("highlight"),
                    v.get("color").and_then(Value::as_str),
                    v.get("note").and_then(Value::as_str),
                    op.hlc.wall_ms as i64
                ],
            )?;
        }
        (Entity::Annotation, "note") => {
            tx.execute(
                "UPDATE annotations SET note=? WHERE id=?",
                params![v.as_str(), id],
            )?;
        }
        (Entity::Annotation, fields::DELETED) => {
            tx.execute(
                "UPDATE annotations SET deleted=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Tag, fields::CREATE) => {
            let name = v.get("name").and_then(Value::as_str).unwrap_or_default();
            tx.execute(
                "INSERT INTO tags(id,name,created_at) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET deleted=0",
                params![id, name, op.hlc.wall_ms as i64],
            )?;
        }
        (Entity::Tag, fields::TITLE) => {
            tx.execute(
                "UPDATE tags SET name=? WHERE id=?",
                params![str_value(v)?, id],
            )?;
        }
        (Entity::Tag, fields::DELETED) => {
            tx.execute(
                "UPDATE tags SET deleted=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::ItemTag, fields::STATE) => {
            let (item_id, tag_id) = id
                .split_once('|')
                .ok_or_else(|| StoreError::Invalid("bad item_tag id".into()))?;
            tx.execute(
                "INSERT INTO item_tags(item_id,tag_id,state) VALUES(?,?,?)
                 ON CONFLICT(item_id,tag_id) DO UPDATE SET state=excluded.state",
                params![item_id, tag_id, str_value(v)?],
            )?;
        }
        (Entity::Series, fields::CREATE) => {
            tx.execute(
                "INSERT OR IGNORE INTO series(id,title,toc_url,created_at,author,cover) VALUES(?,?,?,?,?,?)",
                params![
                    id,
                    v.get("title").and_then(Value::as_str).unwrap_or("Series"),
                    v.get("toc_url").and_then(Value::as_str),
                    op.hlc.wall_ms as i64,
                    v.get("author").and_then(Value::as_str),
                    v.get("cover").and_then(Value::as_str)
                ],
            )?;
        }
        (Entity::Series, fields::TITLE) => {
            tx.execute(
                "UPDATE series SET title=? WHERE id=?",
                params![str_value(v)?, id],
            )?;
        }
        (Entity::Series, fields::DELETED) => {
            tx.execute(
                "UPDATE series SET deleted=? WHERE id=?",
                params![bool_value(v)?, id],
            )?;
        }
        (Entity::Source, fields::FULL_TEXT) => {
            tx.execute(
                "UPDATE sources SET full_text=? WHERE id=?",
                params![str_value(v)?, id],
            )?;
        }
        (Entity::Source, fields::CREATE) => {
            let s: eddy_model::Source = serde_json::from_value(v.clone())?;
            tx.execute(
                "INSERT INTO sources(id,account_id,upstream_id,kind,title,category,site_url,full_text,truncated,updated_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET title=excluded.title, category=excluded.category,
                   site_url=excluded.site_url, truncated=excluded.truncated, updated_at=excluded.updated_at",
                params![s.id, s.account_id, s.upstream_id, s.kind.as_str(), s.title, s.category, s.site_url, s.full_text.as_str(), s.truncated, op.hlc.wall_ms as i64],
            )?;
        }
        (Entity::Source, fields::DELETED) => {
            if bool_value(v)? {
                tx.execute("DELETE FROM sources WHERE id=?", [id])?;
            }
        }
        (Entity::ContentVersion, fields::CREATE) => {
            let cv: eddy_model::ContentVersion = serde_json::from_value(v.clone())?;
            tx.execute(
                "INSERT OR IGNORE INTO content_versions(hash,item_id,origin,source_url,raw_blob,raw_mime,doc_blob,doc_schema,text_len,created_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?)",
                params![cv.hash, cv.item_id, cv.origin.as_str(), cv.source_url, cv.raw_blob, cv.raw_mime, cv.doc_blob, cv.doc_schema, cv.text_len, cv.created_at],
            )?;
        }
        (Entity::Item, "meta") => {
            tx.execute(
                "UPDATE items SET excerpt=?, text_len=?, lang=COALESCE(?,lang) WHERE id=?",
                params![
                    v.get("excerpt").and_then(Value::as_str),
                    v.get("text_len").and_then(Value::as_i64).unwrap_or(0),
                    v.get("lang").and_then(Value::as_str),
                    id
                ],
            )?;
        }
        (Entity::Artifact, fields::CREATE) => {
            let get = |k: &str| v.get(k).and_then(Value::as_str);
            tx.execute(
                "INSERT INTO ai_artifacts(key,item_id,kind,content_hash,lang,model,payload,sync,uploaded,created_at)
                 VALUES(?,?,?,?,?,?,?,1,1,?)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload",
                params![id, get("item_id"), get("kind"), get("content_hash"), get("lang"), get("model"), get("payload"), op.hlc.wall_ms as i64],
            )?;
        }
        (Entity::Setting, fields::VALUE) => {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![id, v.to_string()],
            )?;
        }
        // Ops from newer clients stay in the log (and keep syncing) even when
        // this version has no effect for them.
        _ => {}
    }
    Ok(())
}
