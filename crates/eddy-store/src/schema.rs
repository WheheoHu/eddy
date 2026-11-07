use rusqlite::Connection;

/// Ordered migrations; `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    // 1: core client schema
    r#"
    CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE accounts(
        id TEXT PRIMARY KEY, kind TEXT NOT NULL, base_url TEXT NOT NULL,
        username TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE sources(
        id TEXT PRIMARY KEY, account_id TEXT, upstream_id TEXT, kind TEXT NOT NULL,
        title TEXT NOT NULL, category TEXT, site_url TEXT,
        full_text TEXT NOT NULL DEFAULT 'auto', truncated INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL);
    CREATE TABLE items(
        id TEXT PRIMARY KEY, source_id TEXT, account_id TEXT, upstream_id TEXT,
        kind TEXT NOT NULL, url TEXT, title TEXT NOT NULL, author TEXT,
        published_at INTEGER NOT NULL, added_at INTEGER NOT NULL,
        read INTEGER NOT NULL DEFAULT 0, starred INTEGER NOT NULL DEFAULT 0,
        series_id TEXT, chapter_index INTEGER, current_version TEXT,
        excerpt TEXT, lang TEXT, text_len INTEGER NOT NULL DEFAULT 0,
        progress REAL NOT NULL DEFAULT 0, finished INTEGER NOT NULL DEFAULT 0,
        last_read_at INTEGER, deleted INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX items_published ON items(published_at DESC);
    CREATE INDEX items_source ON items(source_id, published_at DESC);
    CREATE INDEX items_upstream ON items(account_id, upstream_id);
    CREATE INDEX items_series ON items(series_id, chapter_index);
    CREATE TABLE content_versions(
        hash TEXT PRIMARY KEY, item_id TEXT NOT NULL, origin TEXT NOT NULL,
        source_url TEXT, raw_blob TEXT NOT NULL, raw_mime TEXT NOT NULL,
        doc_blob TEXT, doc_schema INTEGER, text_len INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL);
    CREATE INDEX content_versions_item ON content_versions(item_id);
    CREATE TABLE blobs(
        key TEXT PRIMARY KEY, size INTEGER NOT NULL, mime TEXT,
        pinned INTEGER NOT NULL DEFAULT 0, last_access INTEGER NOT NULL);
    CREATE TABLE ops(
        seq INTEGER PRIMARY KEY AUTOINCREMENT, op_id TEXT NOT NULL UNIQUE,
        hlc TEXT NOT NULL, entity TEXT NOT NULL, entity_id TEXT NOT NULL,
        field TEXT NOT NULL, value TEXT NOT NULL, origin TEXT NOT NULL,
        uploaded INTEGER NOT NULL DEFAULT 0, upstream_pending INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX ops_pending ON ops(upstream_pending) WHERE upstream_pending = 1;
    CREATE INDEX ops_unuploaded ON ops(uploaded) WHERE uploaded = 0;
    CREATE TABLE field_clock(
        entity TEXT NOT NULL, entity_id TEXT NOT NULL, field TEXT NOT NULL,
        hlc TEXT NOT NULL, PRIMARY KEY(entity, entity_id, field));
    CREATE TABLE progress(
        item_id TEXT PRIMARY KEY, locator TEXT NOT NULL, progression REAL NOT NULL,
        updated_at INTEGER NOT NULL, device TEXT);
    CREATE TABLE annotations(
        id TEXT PRIMARY KEY, item_id TEXT NOT NULL, locator TEXT NOT NULL,
        kind TEXT NOT NULL, color TEXT, note TEXT, created_at INTEGER NOT NULL,
        deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE tags(
        id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL,
        deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE item_tags(
        item_id TEXT NOT NULL, tag_id TEXT NOT NULL, state TEXT NOT NULL,
        PRIMARY KEY(item_id, tag_id));
    CREATE TABLE tag_suggestions(
        item_id TEXT NOT NULL, name TEXT NOT NULL, PRIMARY KEY(item_id, name));
    CREATE TABLE series(
        id TEXT PRIMARY KEY, title TEXT NOT NULL, toc_url TEXT,
        created_at INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE ai_artifacts(
        key TEXT PRIMARY KEY, item_id TEXT NOT NULL, kind TEXT NOT NULL,
        content_hash TEXT NOT NULL, lang TEXT, model TEXT, payload TEXT NOT NULL,
        sync INTEGER NOT NULL DEFAULT 0, uploaded INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL);
    CREATE INDEX ai_artifacts_item ON ai_artifacts(item_id, kind);
    CREATE TABLE ai_usage(day TEXT PRIMARY KEY, count INTEGER NOT NULL);
    "#,
];

pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}
