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
