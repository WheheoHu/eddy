use eddy_model::{
    ContentOrigin, ContentVersion, FullTextMode, Item, ItemKind, Locator, Series, Source,
    SourceKind, now_ms,
};
use rusqlite::{OptionalExtension, Row, params, params_from_iter, types::Value};

use crate::{Result, Store};

#[derive(Clone, Debug, PartialEq)]
pub enum ItemFilter {
    /// Subscription articles and saved pages (chapters live in their series).
    All,
    Starred,
    Saved,
    /// Started but not finished, most recently read first.
    ContinueReading,
    Source(String),
    Category(String),
    Series(String),
    Tag(String),
    Ids(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct ItemQuery {
    pub filter: ItemFilter,
    pub unread_only: bool,
    pub limit: u32,
    pub offset: u32,
}

impl Default for ItemQuery {
    fn default() -> Self {
        ItemQuery {
            filter: ItemFilter::All,
            unread_only: false,
            limit: 200,
            offset: 0,
        }
    }
}

/// An item joined with its source title for list display.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRow {
    pub item: Item,
    pub source_title: Option<String>,
}

const ITEM_COLUMNS: &str =
    "i.id, i.source_id, i.account_id, i.upstream_id, i.kind, i.url, i.title, i.author,
    i.published_at, i.added_at, i.read, i.starred, i.series_id, i.chapter_index, i.current_version,
    i.excerpt, i.lang, i.text_len, i.progress, i.finished, i.last_read_at, s.title";

fn item_row(r: &Row) -> rusqlite::Result<ItemRow> {
    let kind: String = r.get(4)?;
    Ok(ItemRow {
        item: Item {
            id: r.get(0)?,
            source_id: r.get(1)?,
            account_id: r.get(2)?,
            upstream_id: r.get(3)?,
            kind: ItemKind::parse(&kind).unwrap_or(ItemKind::Feed),
            url: r.get(5)?,
            title: r.get(6)?,
            author: r.get(7)?,
            published_at: r.get(8)?,
            added_at: r.get(9)?,
            read: r.get(10)?,
            starred: r.get(11)?,
            series_id: r.get(12)?,
            chapter_index: r.get(13)?,
            current_version: r.get(14)?,
            excerpt: r.get(15)?,
            lang: r.get(16)?,
            text_len: r.get(17)?,
            progress: r.get(18)?,
            finished: r.get(19)?,
            last_read_at: r.get(20)?,
        },
        source_title: r.get(21)?,
    })
}

fn source_row(r: &Row) -> rusqlite::Result<Source> {
    let kind: String = r.get(3)?;
    let full_text: String = r.get(7)?;
    Ok(Source {
        id: r.get(0)?,
        account_id: r.get(1)?,
        upstream_id: r.get(2)?,
        kind: if kind == "saved" {
            SourceKind::Saved
        } else {
            SourceKind::Feed
        },
        title: r.get(4)?,
        category: r.get(5)?,
        site_url: r.get(6)?,
        full_text: FullTextMode::parse(&full_text),
        truncated: r.get(8)?,
    })
}

fn version_row(r: &Row) -> rusqlite::Result<ContentVersion> {
    let origin: String = r.get(2)?;
    Ok(ContentVersion {
        hash: r.get(0)?,
        item_id: r.get(1)?,
        origin: ContentOrigin::parse(&origin).unwrap_or(ContentOrigin::Feed),
        source_url: r.get(3)?,
        raw_blob: r.get(4)?,
        raw_mime: r.get(5)?,
        doc_blob: r.get(6)?,
        doc_schema: r.get(7)?,
        text_len: r.get(8)?,
        created_at: r.get(9)?,
    })
}

impl Store {
    /// Inserts or refreshes a source's metadata; user policy (full-text mode)
    /// and detection results are preserved.
    pub fn upsert_source(&self, s: &Source) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO sources(id,account_id,upstream_id,kind,title,category,site_url,full_text,truncated,updated_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET title=excluded.title, category=excluded.category,
                   site_url=excluded.site_url, updated_at=excluded.updated_at",
                params![
                    s.id, s.account_id, s.upstream_id, s.kind.as_str(), s.title, s.category, s.site_url,
                    s.full_text.as_str(), s.truncated, now_ms()
                ],
            )?;
            Ok(())
        })
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id,account_id,upstream_id,kind,title,category,site_url,full_text,truncated FROM sources
                 ORDER BY kind DESC, COALESCE(category,''), title COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], source_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn source(&self, id: &str) -> Result<Option<Source>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id,account_id,upstream_id,kind,title,category,site_url,full_text,truncated FROM sources WHERE id=?",
                [id],
                source_row,
            )
            .optional()?)
        })
    }

    pub fn remove_sources_not_in(&self, account_id: &str, keep: &[String]) -> Result<()> {
        self.with(|c| {
            let placeholders = vec!["?"; keep.len()].join(",");
            let sql = if keep.is_empty() {
                "DELETE FROM sources WHERE account_id=?".to_string()
            } else {
                format!("DELETE FROM sources WHERE account_id=? AND id NOT IN ({placeholders})")
            };
            let mut values: Vec<Value> = vec![Value::Text(account_id.to_string())];
            values.extend(keep.iter().map(|k| Value::Text(k.clone())));
            c.execute(&sql, params_from_iter(values))?;
            Ok(())
        })
    }

    pub fn set_source_truncated(&self, id: &str, truncated: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE sources SET truncated=? WHERE id=?",
                params![truncated, id],
            )?;
            Ok(())
        })
    }

    /// Per-source totals for library views: (source_id, total, unread,
    /// latest published_at, latest last_read_at). Chapters are excluded.
    #[allow(clippy::type_complexity)]
    pub fn source_overview(&self) -> Result<Vec<(String, u32, u32, Option<i64>, Option<i64>)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT s.id, COUNT(i.id), COALESCE(SUM(CASE WHEN i.read=0 THEN 1 ELSE 0 END),0),
                        MAX(i.published_at), MAX(i.last_read_at)
                 FROM sources s LEFT JOIN items i ON i.source_id=s.id AND i.deleted=0 AND i.kind!='chapter'
                 GROUP BY s.id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Unread counts per source (id → count).
    pub fn unread_counts(&self) -> Result<Vec<(String, u32)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT source_id, COUNT(*) FROM items WHERE read=0 AND deleted=0 AND kind!='chapter' AND source_id IS NOT NULL GROUP BY source_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Inserts an item if it does not exist yet. Returns whether it was new.
    pub fn insert_item(&self, item: &Item) -> Result<bool> {
        self.with(|c| Ok(insert_item(c, item)? > 0))
    }

    /// Refreshes metadata of an existing item from its upstream.
    pub fn update_item_meta(
        &self,
        id: &str,
        title: &str,
        url: Option<&str>,
        author: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE items SET title=?, url=COALESCE(?,url), author=COALESCE(?,author) WHERE id=?", params![title, url, author, id])?;
            Ok(())
        })
    }

    pub fn item(&self, id: &str) -> Result<Option<ItemRow>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM items i LEFT JOIN sources s ON s.id=i.source_id WHERE i.id=?"),
                [id],
                item_row,
            )
            .optional()?)
        })
    }

    pub fn items(&self, q: &ItemQuery) -> Result<Vec<ItemRow>> {
        let mut clauses = vec!["i.deleted=0".to_string()];
        let mut values: Vec<Value> = Vec::new();
        let mut order = "i.published_at DESC, i.id";
        match &q.filter {
            ItemFilter::All => clauses.push("i.kind!='chapter'".into()),
            ItemFilter::Starred => clauses.push("i.starred=1".into()),
            ItemFilter::Saved => clauses.push("i.kind='saved'".into()),
            ItemFilter::ContinueReading => {
                clauses.push("i.progress>=0.05 AND i.progress<0.95 AND i.finished=0".into());
                order = "i.last_read_at DESC, i.id";
            }
            ItemFilter::Source(id) => {
                clauses.push("i.source_id=?".into());
                values.push(Value::Text(id.clone()));
            }
            ItemFilter::Category(name) => {
                clauses.push("s.category=?".into());
                values.push(Value::Text(name.clone()));
            }
            ItemFilter::Series(id) => {
                clauses.push("i.series_id=?".into());
                values.push(Value::Text(id.clone()));
                order = "i.chapter_index ASC, i.id";
            }
            ItemFilter::Tag(tag) => {
                clauses.push("EXISTS(SELECT 1 FROM item_tags t WHERE t.item_id=i.id AND t.tag_id=? AND t.state IN ('confirmed','auto'))".into());
                values.push(Value::Text(tag.clone()));
            }
            ItemFilter::Ids(ids) => {
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                clauses.push(format!("i.id IN ({})", vec!["?"; ids.len()].join(",")));
                values.extend(ids.iter().map(|i| Value::Text(i.clone())));
            }
        }
        if q.unread_only {
            clauses.push("i.read=0".into());
        }
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM items i LEFT JOIN sources s ON s.id=i.source_id WHERE {} ORDER BY {order} LIMIT {} OFFSET {}",
            clauses.join(" AND "),
            q.limit,
            q.offset
        );
        let mut rows = self.with(|c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(values), item_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        if let ItemFilter::Ids(ids) = &q.filter {
            rows.sort_by_key(|r| ids.iter().position(|id| id == &r.item.id));
        }
        Ok(rows)
    }

    /// Items of an upstream account: (id, upstream_id, read, starred).
    pub fn upstream_items(&self, account_id: &str) -> Result<Vec<(String, String, bool, bool)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, upstream_id, read, starred FROM items WHERE account_id=? AND upstream_id IS NOT NULL AND deleted=0",
            )?;
            let rows = stmt.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Applies read/starred state reported by the upstream service, unless a
    /// local change to that field is still waiting to be pushed: a change made
    /// while a refresh was in flight must win over the stale copy.
    pub fn set_state_from_upstream(&self, item_id: &str, field: &str, value: bool) -> Result<bool> {
        let column = match field {
            eddy_model::fields::READ => "read",
            eddy_model::fields::STARRED => "starred",
            _ => return Ok(false),
        };
        self.with(|c| {
            let pending: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM ops WHERE entity='item' AND entity_id=? AND field=? AND upstream_pending=1)",
                params![item_id, field],
                |r| r.get(0),
            )?;
            if pending {
                return Ok(false);
            }
            let n = c.execute(&format!("UPDATE items SET {column}=? WHERE id=? AND {column}!=?"), params![value, item_id, value])?;
            Ok(n > 0)
        })
    }

    /// Whether upstream state for a field differs from ours with no local
    /// change waiting to be pushed (servers turn such changes into ops).
    pub fn upstream_state_differs(&self, item_id: &str, field: &str, value: bool) -> Result<bool> {
        let column = match field {
            eddy_model::fields::READ => "read",
            eddy_model::fields::STARRED => "starred",
            _ => return Ok(false),
        };
        self.with(|c| {
            let differs: bool = c.query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 AND {column}!=?2)
                     AND NOT EXISTS(SELECT 1 FROM ops WHERE entity='item' AND entity_id=?1 AND field=?3 AND upstream_pending=1)"
                ),
                params![item_id, value, field],
                |r| r.get(0),
            )?;
            Ok(differs)
        })
    }

    pub fn set_current_version(
        &self,
        item_id: &str,
        hash: &str,
        excerpt: &str,
        text_len: u32,
        lang: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE items SET current_version=?, excerpt=?, text_len=?, lang=COALESCE(?,lang) WHERE id=?",
                params![hash, excerpt, text_len, lang, item_id],
            )?;
            Ok(())
        })
    }

    pub fn insert_content_version(&self, v: &ContentVersion) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO content_versions(hash,item_id,origin,source_url,raw_blob,raw_mime,doc_blob,doc_schema,text_len,created_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(hash) DO UPDATE SET doc_blob=COALESCE(excluded.doc_blob,doc_blob),
                   doc_schema=COALESCE(excluded.doc_schema,doc_schema), text_len=excluded.text_len",
                params![
                    v.hash, v.item_id, v.origin.as_str(), v.source_url, v.raw_blob, v.raw_mime, v.doc_blob, v.doc_schema,
                    v.text_len, v.created_at
                ],
            )?;
            Ok(())
        })
    }

    pub fn content_version(&self, hash: &str) -> Result<Option<ContentVersion>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT hash,item_id,origin,source_url,raw_blob,raw_mime,doc_blob,doc_schema,text_len,created_at
                 FROM content_versions WHERE hash=?",
                [hash],
                version_row,
            )
            .optional()?)
        })
    }

    pub fn content_versions(&self, item_id: &str) -> Result<Vec<ContentVersion>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT hash,item_id,origin,source_url,raw_blob,raw_mime,doc_blob,doc_schema,text_len,created_at
                 FROM content_versions WHERE item_id=? ORDER BY created_at",
            )?;
            let rows = stmt.query_map([item_id], version_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn set_doc_blob(
        &self,
        hash: &str,
        doc_blob: &str,
        schema: u32,
        text_len: u32,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE content_versions SET doc_blob=?, doc_schema=?, text_len=? WHERE hash=?",
                params![doc_blob, schema, text_len, hash],
            )?;
            Ok(())
        })
    }

    pub fn progress(&self, item_id: &str) -> Result<Option<Locator>> {
        let raw: Option<String> = self.with(|c| {
            Ok(c.query_row(
                "SELECT locator FROM progress WHERE item_id=?",
                [item_id],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn series_list(&self) -> Result<Vec<(Series, u32, u32)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT s.id, s.title, s.toc_url, s.created_at,
                   (SELECT COUNT(*) FROM items i WHERE i.series_id=s.id AND i.deleted=0),
                   (SELECT COUNT(*) FROM items i WHERE i.series_id=s.id AND i.deleted=0 AND i.finished=1)
                 FROM series s WHERE s.deleted=0 ORDER BY s.created_at DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((Series { id: r.get(0)?, title: r.get(1)?, toc_url: r.get(2)?, created_at: r.get(3)? }, r.get(4)?, r.get(5)?))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn series(&self, id: &str) -> Result<Option<Series>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id,title,toc_url,created_at FROM series WHERE id=? AND deleted=0",
                [id],
                |r| {
                    Ok(Series {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        toc_url: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()?)
        })
    }

    /// Tags as (id, name), excluding deleted ones.
    pub fn tags(&self) -> Result<Vec<(String, String)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name FROM tags WHERE deleted=0 ORDER BY name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Tags with the number of items carrying them: (id, name, count).
    pub fn tag_counts(&self) -> Result<Vec<(String, String, u32)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT t.id, t.name, (SELECT COUNT(*) FROM item_tags it JOIN items i ON i.id=it.item_id
                    WHERE it.tag_id=t.id AND it.state IN ('confirmed','auto') AND i.deleted=0)
                 FROM tags t WHERE t.deleted=0 ORDER BY t.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Item tag states: (tag_id, state).
    pub fn item_tag_states(&self, item_id: &str) -> Result<Vec<(String, String)>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT tag_id, state FROM item_tags WHERE item_id=?")?;
            let rows = stmt.query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// Tag IDs attached to an item with the given state.
    pub fn item_tags(&self, item_id: &str, state: &str) -> Result<Vec<String>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT tag_id FROM item_tags WHERE item_id=? AND state=?")?;
            let rows = stmt.query_map(params![item_id, state], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn set_tag_suggestions(&self, item_id: &str, names: &[String]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM tag_suggestions WHERE item_id=?", [item_id])?;
            for name in names {
                tx.execute(
                    "INSERT OR IGNORE INTO tag_suggestions(item_id,name) VALUES(?,?)",
                    params![item_id, name],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn tag_suggestions(&self, item_id: &str) -> Result<Vec<String>> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT name FROM tag_suggestions WHERE item_id=? ORDER BY name")?;
            let rows = stmt.query_map([item_id], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_ai_artifact(
        &self,
        key: &str,
        item_id: &str,
        kind: &str,
        content_hash: &str,
        lang: Option<&str>,
        model: Option<&str>,
        payload: &str,
        sync: bool,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO ai_artifacts(key,item_id,kind,content_hash,lang,model,payload,sync,uploaded,created_at)
                 VALUES(?,?,?,?,?,?,?,?,0,?)
                 ON CONFLICT(key) DO UPDATE SET payload=excluded.payload, sync=excluded.sync, uploaded=0, created_at=excluded.created_at",
                params![key, item_id, kind, content_hash, lang, model, payload, sync, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn ai_artifact(&self, key: &str) -> Result<Option<String>> {
        self.with(|c| {
            Ok(
                c.query_row("SELECT payload FROM ai_artifacts WHERE key=?", [key], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
    }

    /// Latest artifact of `kind` for an item: (content_hash, lang, payload).
    pub fn latest_ai_artifact(
        &self,
        item_id: &str,
        kind: &str,
    ) -> Result<Option<(String, Option<String>, String)>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT content_hash, lang, payload FROM ai_artifacts WHERE item_id=? AND kind=? ORDER BY created_at DESC LIMIT 1",
                params![item_id, kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
        })
    }

    /// Synced artifacts not yet uploaded: (key, item_id, kind, content_hash, lang, model, payload).
    #[allow(clippy::type_complexity)]
    pub fn unuploaded_ai_artifacts(
        &self,
    ) -> Result<
        Vec<(
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        )>,
    > {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT key,item_id,kind,content_hash,lang,model,payload FROM ai_artifacts WHERE sync=1 AND uploaded=0",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn mark_ai_artifacts_uploaded(&self, keys: &[String]) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            for key in keys {
                tx.execute("UPDATE ai_artifacts SET uploaded=1 WHERE key=?", [key])?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}

pub(crate) fn insert_item(c: &rusqlite::Connection, item: &Item) -> rusqlite::Result<usize> {
    c.execute(
        "INSERT OR IGNORE INTO items(id,source_id,account_id,upstream_id,kind,url,title,author,published_at,added_at,
            read,starred,series_id,chapter_index,current_version,excerpt,lang,text_len,progress,finished,last_read_at)
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            item.id, item.source_id, item.account_id, item.upstream_id, item.kind.as_str(), item.url, item.title,
            item.author, item.published_at, item.added_at, item.read, item.starred, item.series_id, item.chapter_index,
            item.current_version, item.excerpt, item.lang, item.text_len, item.progress, item.finished, item.last_read_at
        ],
    )
}
