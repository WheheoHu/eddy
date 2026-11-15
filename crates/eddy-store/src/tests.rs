use eddy_model::{
    ContentOrigin, ContentVersion, Entity, Hlc, Item, ItemKind, Locator, Op, Position, TextQuote,
    fields,
};
use serde_json::json;

use super::*;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn item(id: &str, published: i64) -> Item {
    Item {
        id: id.into(),
        source_id: Some("src".into()),
        account_id: Some("acct".into()),
        upstream_id: Some(format!("up-{id}")),
        kind: ItemKind::Feed,
        url: Some(format!("https://example.com/{id}")),
        title: id.into(),
        author: None,
        published_at: published,
        added_at: published,
        read: false,
        starred: false,
        series_id: None,
        chapter_index: None,
        chapter_level: 0,
        current_version: None,
        excerpt: None,
        lang: None,
        text_len: 0,
        progress: 0.0,
        finished: false,
        last_read_at: None,
    }
}

fn op(n: u64, node: &str, entity: Entity, id: &str, field: &str, value: serde_json::Value) -> Op {
    Op {
        op_id: format!("{node}-{n}-{field}"),
        hlc: Hlc {
            wall_ms: n,
            counter: 0,
            node: node.into(),
        },
        entity,
        entity_id: id.into(),
        field: field.into(),
        value,
    }
}

#[test]
fn reopen_keeps_data_and_device_id() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let s = Store::open(dir.path()).unwrap();
        s.insert_item(&item("a", 1)).unwrap();
        s.device_id().unwrap()
    };
    let s = Store::open(dir.path()).unwrap();
    assert_eq!(s.device_id().unwrap(), id);
    assert!(s.item("a").unwrap().is_some());
}

#[test]
fn last_writer_wins_per_field_and_duplicates_are_ignored() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    s.record_local(
        &op(10, "dev1", Entity::Item, "a", fields::STARRED, json!(true)),
        false,
    )
    .unwrap();
    // Older remote write loses, newer one wins, a replay is ignored.
    assert!(
        !s.apply_remote(&op(
            5,
            "dev2",
            Entity::Item,
            "a",
            fields::STARRED,
            json!(false)
        ))
        .unwrap()
    );
    assert!(s.item("a").unwrap().unwrap().item.starred);
    let newer = op(20, "dev2", Entity::Item, "a", fields::STARRED, json!(false));
    assert!(s.apply_remote(&newer).unwrap());
    assert!(!s.apply_remote(&newer).unwrap());
    assert!(!s.item("a").unwrap().unwrap().item.starred);
    // Fields are independent.
    assert!(
        s.apply_remote(&op(1, "dev2", Entity::Item, "a", fields::READ, json!(true)))
            .unwrap()
    );
    assert!(s.item("a").unwrap().unwrap().item.read);
    // Remote ops are not re-uploaded; the local one is.
    let pending = s.unuploaded_ops(100).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].op.hlc.node, "dev1");
}

#[test]
fn upstream_state_does_not_clobber_pending_local_changes() {
    let (_d, s) = store();
    s.insert_item(&item("recent", 30)).unwrap();
    s.insert_item(&item("older", 20)).unwrap();
    s.record_local(
        &op(1, "dev", Entity::Item, "recent", fields::READ, json!(true)),
        true,
    )
    .unwrap();
    // A refresh started before the local change still reports it unread.
    assert!(
        !s.set_state_from_upstream("recent", fields::READ, false)
            .unwrap()
    );
    assert!(s.item("recent").unwrap().unwrap().item.read);
    assert!(
        s.set_state_from_upstream("older", fields::READ, true)
            .unwrap()
    );
    // After the push succeeds, upstream is authoritative again.
    let seqs: Vec<i64> = s
        .upstream_pending()
        .unwrap()
        .iter()
        .map(|o| o.seq)
        .collect();
    s.clear_upstream_pending(&seqs).unwrap();
    assert!(
        s.set_state_from_upstream("recent", fields::READ, false)
            .unwrap()
    );
}

#[test]
fn item_queries_filter_and_order() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    s.insert_item(&item("b", 3)).unwrap();
    let mut c = item("c", 2);
    c.kind = ItemKind::Saved;
    s.insert_item(&c).unwrap();
    let mut ch = item("ch", 9);
    ch.kind = ItemKind::Chapter;
    s.insert_item(&ch).unwrap();
    s.record_local(
        &op(1, "d", Entity::Item, "b", fields::READ, json!(true)),
        false,
    )
    .unwrap();
    s.record_local(
        &op(2, "d", Entity::Item, "a", fields::STARRED, json!(true)),
        false,
    )
    .unwrap();

    let ids = |q: ItemQuery| {
        s.items(&q)
            .unwrap()
            .into_iter()
            .map(|r| r.item.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(ItemQuery::default()), ["b", "c", "a"]);
    assert_eq!(
        ids(ItemQuery {
            unread_only: true,
            ..Default::default()
        }),
        ["c", "a"]
    );
    assert_eq!(
        ids(ItemQuery {
            unread_only: true,
            keep: vec!["b".into()],
            ..Default::default()
        }),
        ["b", "c", "a"]
    );
    assert_eq!(
        ids(ItemQuery {
            filter: ItemFilter::Saved,
            unread_only: true,
            keep: vec!["b".into()],
            ..Default::default()
        }),
        ["c"]
    );
    assert_eq!(
        ids(ItemQuery {
            filter: ItemFilter::Starred,
            ..Default::default()
        }),
        ["a"]
    );
    assert_eq!(
        ids(ItemQuery {
            filter: ItemFilter::Saved,
            ..Default::default()
        }),
        ["c"]
    );
    assert_eq!(
        ids(ItemQuery {
            filter: ItemFilter::Ids(vec!["a".into(), "ch".into()]),
            ..Default::default()
        }),
        ["a", "ch"]
    );
    // Titles are the IDs; the chapter "ch" is left to its series.
    let titled = |text: &str| {
        ids(ItemQuery {
            filter: ItemFilter::Title(text.into()),
            ..Default::default()
        })
    };
    assert_eq!(titled("C"), ["c"]);
    assert!(titled("%").is_empty());
    assert!(titled("  ").is_empty());

    let loc = |p: f64| Locator {
        content_hash: "h".into(),
        position: Position::Block {
            block_id: "b1".into(),
            offset: 0,
        },
        quote: TextQuote::default(),
        progression: p,
    };
    s.record_local(
        &op(
            3,
            "d",
            Entity::Progress,
            "a",
            fields::LOCATOR,
            serde_json::to_value(loc(0.4)).unwrap(),
        ),
        false,
    )
    .unwrap();
    s.record_local(
        &op(
            4,
            "d",
            Entity::Progress,
            "c",
            fields::LOCATOR,
            serde_json::to_value(loc(0.97)).unwrap(),
        ),
        false,
    )
    .unwrap();
    assert_eq!(
        ids(ItemQuery {
            filter: ItemFilter::ContinueReading,
            ..Default::default()
        }),
        ["a"]
    );
    assert_eq!(
        s.count_items(&ItemQuery {
            filter: ItemFilter::ContinueReading,
            limit: 0,
            ..Default::default()
        })
        .unwrap(),
        1
    );
    assert_eq!(
        s.count_items(&ItemQuery {
            filter: ItemFilter::Ids(vec![]),
            ..Default::default()
        })
        .unwrap(),
        0
    );
    assert_eq!(s.progress("a").unwrap().unwrap().progression, 0.4);
}

#[test]
fn tags_series_settings_and_sources_via_ops() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    s.record_local(
        &op(
            1,
            "d",
            Entity::Tag,
            "t1",
            fields::CREATE,
            json!({"name": "Rendering"}),
        ),
        false,
    )
    .unwrap();
    s.record_local(
        &op(
            2,
            "d",
            Entity::ItemTag,
            "a|t1",
            fields::STATE,
            json!("confirmed"),
        ),
        false,
    )
    .unwrap();
    assert_eq!(s.tags().unwrap(), vec![("t1".into(), "Rendering".into())]);
    assert_eq!(
        s.items(&ItemQuery {
            filter: ItemFilter::Tag("t1".into()),
            ..Default::default()
        })
        .unwrap()
        .len(),
        1
    );

    s.record_local(
        &op(
            3,
            "d",
            Entity::Series,
            "s1",
            fields::CREATE,
            json!({"title": "Book", "toc_url": null}),
        ),
        false,
    )
    .unwrap();
    s.record_local(
        &op(
            4,
            "d",
            Entity::Item,
            "a",
            fields::SERIES,
            json!({"series_id": "s1", "chapter_index": 2}),
        ),
        false,
    )
    .unwrap();
    let series = s.series_list().unwrap();
    assert_eq!(series[0].0.title, "Book");
    assert_eq!(series[0].1, 1);

    s.record_local(
        &op(
            5,
            "d",
            Entity::Setting,
            "ai",
            fields::VALUE,
            json!({"model": "m"}),
        ),
        false,
    )
    .unwrap();
    assert_eq!(s.setting("ai").unwrap(), Some(json!({"model": "m"})));
}

#[test]
fn gallery_lists_marked_sources_and_items_without_known_empty_ones() {
    let (_d, s) = store();
    let source = |id: &str| eddy_model::Source {
        id: id.into(),
        account_id: Some("acct".into()),
        upstream_id: None,
        kind: eddy_model::SourceKind::Feed,
        title: id.into(),
        category: Some("A".into()),
        site_url: None,
        full_text: eddy_model::FullTextMode::Auto,
        truncated: false,
        gallery: false,
    };
    s.upsert_source(&source("src")).unwrap();
    s.upsert_source(&source("other")).unwrap();
    for (id, src, t) in [("a", "src", 3), ("b", "src", 2), ("c", "other", 1)] {
        s.insert_item(&Item {
            source_id: Some(src.into()),
            ..item(id, t)
        })
        .unwrap();
    }

    s.record_local(
        &op(1, "d", Entity::Source, "src", fields::GALLERY, json!(true)),
        false,
    )
    .unwrap();
    assert!(s.source("src").unwrap().unwrap().gallery);
    // Metadata refreshes from the upstream keep the mark.
    s.upsert_source(&source("src")).unwrap();
    assert!(s.source("src").unwrap().unwrap().gallery);

    let gallery = |sources: &[&str], items: &[&str]| {
        s.items(&ItemQuery {
            filter: ItemFilter::Gallery {
                sources: sources.iter().map(|x| x.to_string()).collect(),
                items: items.iter().map(|x| x.to_string()).collect(),
            },
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect::<Vec<_>>()
    };
    assert_eq!(gallery(&["src"], &["c"]), ["a", "b", "c"]);
    assert!(gallery(&[], &[]).is_empty());

    s.put_media_set("b", "{}", "no_media").unwrap();
    s.put_media_set("a", "{\"items\":[]}", "ready").unwrap();
    assert_eq!(gallery(&["src"], &[]), ["a"]);
    let (json, status, _) = s.media_set("a").unwrap().unwrap();
    assert_eq!(
        (json.as_str(), status.as_str()),
        ("{\"items\":[]}", "ready")
    );
}

#[test]
fn blobs_round_trip_and_eviction_protects_unread_content() {
    let (_d, s) = store();
    let mut read_item = item("read", 1);
    read_item.read = true;
    s.insert_item(&read_item).unwrap();
    s.insert_item(&item("unread", 2)).unwrap();
    let old = s.put_blob(&vec![1u8; 1000], "text/html", false).unwrap();
    let keep = s.put_blob(&vec![2u8; 1000], "text/html", false).unwrap();
    let pinned = s.put_blob(&vec![3u8; 1000], "text/html", true).unwrap();
    for (item, blob) in [("read", &old), ("unread", &keep)] {
        s.insert_content_version(&ContentVersion {
            hash: format!("v-{item}"),
            item_id: item.into(),
            origin: ContentOrigin::Feed,
            source_url: None,
            raw_blob: blob.clone(),
            raw_mime: "text/html".into(),
            doc_blob: None,
            doc_schema: None,
            text_len: 0,
            created_at: 0,
        })
        .unwrap();
    }
    assert_eq!(s.get_blob(&old).unwrap().unwrap().len(), 1000);
    assert_eq!(s.blob_usage().unwrap(), 3000);
    let freed = s.evict(500).unwrap();
    assert_eq!(freed, 1000);
    assert!(!s.has_blob(&old).unwrap());
    assert!(s.has_blob(&keep).unwrap());
    assert!(s.has_blob(&pinned).unwrap());
    assert!(s.put_blob_keyed("../escape", b"x", "x", false).is_err());
}

#[test]
fn deleted_items_hide_everywhere_and_release_files_only_they_use() {
    let (_d, s) = store();
    for id in ["gone", "kept"] {
        let mut saved = item(id, 1);
        saved.kind = ItemKind::Saved;
        s.insert_item(&saved).unwrap();
    }
    let book = s
        .put_blob(b"the whole book", "application/epub+zip", true)
        .unwrap();
    let only_gone = s
        .put_blob(b"gone's document", "application/json", false)
        .unwrap();
    for (item_id, doc) in [("gone", Some(&only_gone)), ("kept", None)] {
        s.insert_content_version(&ContentVersion {
            hash: format!("v-{item_id}"),
            item_id: item_id.into(),
            origin: ContentOrigin::Imported,
            source_url: None,
            raw_blob: book.clone(),
            raw_mime: "application/epub+zip".into(),
            doc_blob: doc.cloned(),
            doc_schema: Some(1),
            text_len: 0,
            created_at: 0,
        })
        .unwrap();
    }

    // A deletion synced from another device.
    assert!(
        s.apply_remote(&op(
            3,
            "dev2",
            Entity::Item,
            "gone",
            fields::DELETED,
            json!(true)
        ))
        .unwrap()
    );
    let saved = s
        .items(&ItemQuery {
            filter: ItemFilter::Saved,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].item.id, "kept");
    assert!(s.items_with_raw_blob(&book).unwrap() == ["kept"]);

    assert_eq!(s.purge_orphan_blobs().unwrap(), 15);
    assert!(!s.has_blob(&only_gone).unwrap());
    assert!(s.has_blob(&book).unwrap(), "still used by a live item");

    s.record_local(
        &op(
            4,
            "dev1",
            Entity::Item,
            "kept",
            fields::DELETED,
            json!(true),
        ),
        false,
    )
    .unwrap();
    s.purge_orphan_blobs().unwrap();
    assert!(
        !s.has_blob(&book).unwrap(),
        "pinned originals go with their last item"
    );
    assert_eq!(s.blob_usage().unwrap(), 0);
}

#[test]
fn freeing_files_never_scans_all_blobs_or_versions() {
    let (_d, s) = store();
    let plan: Vec<String> = s
        .with(|c| {
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {}", blobs::ORPHAN_BLOBS))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .unwrap();
    let scanned: Vec<&str> = plan
        .iter()
        .filter_map(|d| d.strip_prefix("SCAN ")?.split_whitespace().next())
        .collect();
    for table in ["b", "cv", "blobs", "content_versions"] {
        assert!(!scanned.contains(&table), "{table} scanned: {plan:#?}");
    }
}

#[test]
fn only_a_winning_change_is_owed_to_the_upstream_service() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    // On a server: the newer change arrives first, the older one after it.
    let newer = op(20, "dev1", Entity::Item, "a", fields::STARRED, json!(false));
    let older = op(10, "dev2", Entity::Item, "a", fields::STARRED, json!(true));
    assert!(s.apply_remote_from(&newer, Some("dev1"), true).unwrap());
    assert!(!s.apply_remote_from(&older, Some("dev2"), true).unwrap());
    let pending = s.upstream_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].op.op_id, newer.op_id);
    assert!(!s.item("a").unwrap().unwrap().item.starred);
}

#[test]
fn upload_bookkeeping_survives_refusals_and_a_new_server() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    let mine = op(10, "me", Entity::Item, "a", fields::READ, json!(true));
    let bad = op(11, "me", Entity::Item, "a", fields::STARRED, json!(true));
    s.record_local(&mine, false).unwrap();
    s.record_local(&bad, false).unwrap();
    s.apply_remote(&op(
        12,
        "other",
        Entity::Tag,
        "t",
        fields::CREATE,
        json!({"name": "T"}),
    ))
    .unwrap();
    assert_eq!(s.upload_counts().unwrap(), (2, 0));

    let seqs: Vec<i64> = s
        .unuploaded_ops(10)
        .unwrap()
        .iter()
        .map(|o| o.seq)
        .collect();
    s.mark_uploaded(&seqs).unwrap();
    s.mark_rejected(std::slice::from_ref(&bad.op_id)).unwrap();
    assert_eq!(s.upload_counts().unwrap(), (0, 1));
    assert!(
        s.unuploaded_ops(10).unwrap().is_empty(),
        "refused ops are not sent again"
    );

    // A changed server gets another try at what was refused...
    s.retry_rejected().unwrap();
    assert_eq!(s.upload_counts().unwrap(), (1, 0));
    // ...and a server this device never synced with gets the whole log,
    // what came from other devices included.
    s.reset_uploaded().unwrap();
    assert_eq!(s.unuploaded_ops(10).unwrap().len(), 3);
    assert_eq!(s.max_hlc().unwrap().unwrap().wall_ms, 12);
}

#[test]
fn leaving_the_server_hands_unsent_flags_to_the_upstream_service() {
    let (_d, s) = store();
    s.insert_item(&item("a", 1)).unwrap();
    s.insert_item(&item("b", 2)).unwrap();
    // Sent to the server already; not owed to anyone.
    s.record_local(
        &op(10, "me", Entity::Item, "a", fields::READ, json!(true)),
        false,
    )
    .unwrap();
    let sent: Vec<i64> = s
        .unuploaded_ops(10)
        .unwrap()
        .iter()
        .map(|o| o.seq)
        .collect();
    s.mark_uploaded(&sent).unwrap();
    // Never sent: star then unstar, and a tag (not FreshRSS's business).
    s.record_local(
        &op(11, "me", Entity::Item, "b", fields::STARRED, json!(true)),
        false,
    )
    .unwrap();
    s.record_local(
        &op(12, "me", Entity::Item, "b", fields::STARRED, json!(false)),
        false,
    )
    .unwrap();
    s.record_local(
        &op(
            13,
            "me",
            Entity::Tag,
            "t",
            fields::CREATE,
            json!({"name": "T"}),
        ),
        false,
    )
    .unwrap();
    s.requeue_unuploaded_for_upstream().unwrap();
    let pending = s.upstream_pending().unwrap();
    assert_eq!(
        pending.len(),
        1,
        "only the current value of the unsent flag"
    );
    assert_eq!(pending[0].op.hlc.wall_ms, 12);
}

#[test]
fn data_from_a_newer_version_is_refused_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("eddy.sqlite3")).unwrap();
    let newer = (SCHEMA_VERSION + 1) as i64;
    conn.pragma_update(None, "user_version", newer).unwrap();
    drop(conn);
    match Store::open(dir.path()) {
        Err(StoreError::Newer { found, supported }) => {
            assert_eq!((found, supported), (SCHEMA_VERSION + 1, SCHEMA_VERSION));
        }
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
    // Nothing was touched: the version is still the newer one.
    let conn = rusqlite::Connection::open(dir.path().join("eddy.sqlite3")).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, newer);
}

#[test]
fn an_older_database_is_migrated_forward() {
    let dir = tempfile::tempdir().unwrap();
    {
        // The first released schema, with a row in it.
        let mut conn = rusqlite::Connection::open(dir.path().join("eddy.sqlite3")).unwrap();
        crate::schema::migrate_to(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO items(id,kind,title,published_at,added_at) VALUES('old','saved','Old',1,1)",
            [],
        )
        .unwrap();
    }
    let s = Store::open(dir.path()).unwrap();
    let row = s.item("old").unwrap().unwrap();
    assert_eq!(row.item.title, "Old");
    assert_eq!(
        row.item.chapter_level, 0,
        "columns added later have defaults"
    );
    assert!(s.devices().unwrap().is_empty(), "tables added later exist");
}

#[test]
fn a_pulled_page_applies_like_its_ops_one_by_one_and_all_or_nothing() {
    let page = |extra: Option<Op>| {
        let mut ops = vec![
            op(
                1,
                "srv",
                Entity::Item,
                "a",
                fields::CREATE,
                serde_json::to_value(item("a", 1)).unwrap(),
            ),
            op(
                2,
                "srv",
                Entity::Item,
                "b",
                fields::CREATE,
                serde_json::to_value(item("b", 2)).unwrap(),
            ),
            op(20, "dev2", Entity::Item, "a", fields::STARRED, json!(true)),
            // Loses to the write above, and the one after it is a replay.
            op(5, "dev3", Entity::Item, "a", fields::STARRED, json!(false)),
            op(20, "dev2", Entity::Item, "a", fields::STARRED, json!(true)),
            op(30, "dev2", Entity::Item, "b", fields::READ, json!(true)),
        ];
        ops.extend(extra);
        ops
    };
    let (_d1, one_by_one) = store();
    let (_d2, paged) = store();
    // Item "a" is already on both devices; "b" is new.
    one_by_one.insert_item(&item("a", 1)).unwrap();
    paged.insert_item(&item("a", 1)).unwrap();

    let expected: Vec<bool> = page(None)
        .iter()
        .map(|o| one_by_one.apply_remote(o).unwrap())
        .collect();
    let got = paged
        .apply_remote_page(&page(None), Some(("cursor", "6")))
        .unwrap();
    assert_eq!(got.iter().map(|r| r.applied).collect::<Vec<_>>(), expected);
    assert_eq!(
        got.iter().map(|r| r.known).collect::<Vec<_>>(),
        [true, false, false, false, false, false]
    );
    assert_eq!(paged.meta_get("cursor").unwrap().as_deref(), Some("6"));
    for id in ["a", "b"] {
        assert_eq!(
            paged.item(id).unwrap().unwrap().item,
            one_by_one.item(id).unwrap().unwrap().item
        );
    }
    assert!(paged.unuploaded_ops(100).unwrap().is_empty());

    // An op that cannot be applied takes the whole page and the cursor with
    // it, so nothing is half done and the page is pulled again.
    let (_d3, broken) = store();
    let bad = op(
        40,
        "srv",
        Entity::Item,
        "c",
        fields::CREATE,
        json!("nonsense"),
    );
    assert!(
        broken
            .apply_remote_page(&page(Some(bad)), Some(("cursor", "7")))
            .is_err()
    );
    assert!(broken.item("b").unwrap().is_none());
    assert_eq!(broken.meta_get("cursor").unwrap(), None);
}
