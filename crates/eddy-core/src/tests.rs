use std::sync::Arc;

use eddy_model::BlockKind;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::*;

pub(crate) async fn mock_freshrss() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=tok\n"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/subscription/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subscriptions": [{"id": "feed/1", "title": "Graphics Blog", "categories": [{"label": "Tech"}]}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .and(query_param("s", "user/-/state/com.google/reading-list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"itemRefs": [{"id": "2"}, {"id": "3"}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .and(query_param("s", "user/-/state/com.google/starred"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"itemRefs": [{"id": "1"}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/api/greader.php/reader/api/0/stream/contents/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [item(1, "Old post", true), item(2, "New post", false)]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/stream/items/contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [item(3, "Unread elsewhere", false)]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("T"))
        .mount(&server)
        .await;
    server
}

fn item(n: u64, title: &str, read: bool) -> serde_json::Value {
    let mut cats = vec!["user/-/state/com.google/reading-list"];
    if read {
        cats.push("user/-/state/com.google/read");
    }
    json!({
        "id": format!("tag:google.com,2005:reader/item/{n:016x}"),
        "title": title,
        "published": 1_700_000_000 + n,
        "categories": cats,
        "alternate": [{"href": format!("https://blog.example/{n}")}],
        "summary": {"content": format!("<h1>{title}</h1><p>Body of {title} with <img src=\"/img/{n}.png\"></p>")},
        "origin": {"streamId": "feed/1", "title": "Graphics Blog"}
    })
}

pub(crate) fn open(dir: &std::path::Path) -> Engine {
    open_with(dir, Arc::new(MemoryPlatform::default()))
}

pub(crate) fn open_with(dir: &std::path::Path, platform: Arc<MemoryPlatform>) -> Engine {
    Engine::open(dir, platform, EngineOptions::default()).unwrap()
}

#[tokio::test]
async fn standalone_refresh_ingests_items_and_reconciles_state() {
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let keychain = Arc::new(MemoryPlatform::default());
    let engine = open_with(dir.path(), keychain.clone());
    assert_eq!(engine.status().unwrap().mode, Mode::Unconfigured);
    assert!(matches!(engine.refresh().await, Err(EngineError::NotConfigured)));

    let account = engine.configure_freshrss(&server.uri(), "me", "secret").await.unwrap();
    assert_eq!(account.base_url, format!("{}/api/greader.php", server.uri()));
    assert_eq!(engine.status().unwrap().mode, Mode::Standalone);

    let summary = engine.refresh().await.unwrap();
    assert_eq!(summary.new_items, 3);
    assert_eq!(summary.sources, 1);

    let rows = engine.items(&ItemQuery::default()).unwrap();
    let titles: Vec<_> = rows.iter().map(|r| r.item.title.as_str()).collect();
    assert_eq!(titles, ["Unread elsewhere", "New post", "Old post"]);
    let old = rows.iter().find(|r| r.item.title == "Old post").unwrap();
    assert!(old.item.read && old.item.starred);
    assert_eq!(old.source_title.as_deref(), Some("Graphics Blog"));
    assert_eq!(engine.sources().unwrap().iter().find(|s| s.source.title == "Graphics Blog").unwrap().unread, 2);

    let doc = engine.document(&old.item.id).await.unwrap().unwrap().document;
    // The leading heading repeats the title and is dropped.
    assert!(matches!(doc.blocks[0].kind, BlockKind::Paragraph { .. }));
    assert!(doc.blocks.iter().any(|b| matches!(&b.kind, BlockKind::Image { src, .. } if src == "https://blog.example/img/1.png")));

    // A second refresh is idempotent.
    assert_eq!(engine.refresh().await.unwrap().new_items, 0);

    // Reopening restores standalone mode from the keychain.
    drop(engine);
    let engine = open_with(dir.path(), keychain);
    assert_eq!(engine.status().unwrap().mode, Mode::Standalone);
}

#[tokio::test]
async fn local_changes_are_pushed_to_freshrss() {
    let server = mock_freshrss().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/edit-tag"))
        .and(body_string_contains("a=user%2F-%2Fstate%2Fcom.google%2Fread"))
        .and(body_string_contains("0000000000000002"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .expect(1..)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine.configure_freshrss(&server.uri(), "me", "secret").await.unwrap();
    engine.refresh().await.unwrap();
    let new_post = engine.items(&ItemQuery::default()).unwrap().into_iter().find(|r| r.item.title == "New post").unwrap();
    engine.set_read(&new_post.item.id, true).await.unwrap();
    assert!(engine.item(&new_post.item.id).unwrap().unwrap().item.read);
    // Refresh pushes pending changes before pulling.
    engine.refresh().await.unwrap();
    assert!(engine.inner.store.upstream_pending().unwrap().is_empty());
}

#[tokio::test]
async fn unpushed_local_changes_survive_a_refresh() {
    // No edit-tag endpoint: every push fails.
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine.configure_freshrss(&server.uri(), "me", "secret").await.unwrap();
    engine.refresh().await.unwrap();
    let new_post = engine.items(&ItemQuery::default()).unwrap().into_iter().find(|r| r.item.title == "New post").unwrap();
    engine.set_read(&new_post.item.id, true).await.unwrap();
    assert!(engine.refresh().await.is_err());
    assert!(engine.item(&new_post.item.id).unwrap().unwrap().item.read);
    assert_eq!(engine.inner.store.upstream_pending().unwrap().len(), 1);
    assert!(engine.status().unwrap().last_error.is_some());
}
