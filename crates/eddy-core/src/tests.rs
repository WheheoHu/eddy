use std::sync::Arc;

use eddy_model::BlockKind;
use serde_json::json;
use wiremock::matchers::{
    body_string_contains, method, path, path_regex, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::*;

pub(crate) async fn mock_freshrss() -> MockServer {
    mock_freshrss_linking("https://blog.example").await
}

/// Mock FreshRSS whose items link to `{article_base}/{n}`.
pub(crate) async fn mock_freshrss_linking(article_base: &str) -> MockServer {
    let server = MockServer::start().await;
    let item = |n: u64, title: &str, read: bool| item_at(article_base, n, title, read);
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
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"itemRefs": [{"id": "2"}, {"id": "3"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .and(query_param("s", "user/-/state/com.google/starred"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"itemRefs": [{"id": "1"}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(
            "^/api/greader.php/reader/api/0/stream/contents/.*",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"items": [item(1, "Old post", true), item(2, "New post", false)]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/stream/items/contents"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [item(3, "Unread elsewhere", false)]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("T"))
        .mount(&server)
        .await;
    server
}

fn item_at(base: &str, n: u64, title: &str, read: bool) -> serde_json::Value {
    let mut cats = vec!["user/-/state/com.google/reading-list"];
    if read {
        cats.push("user/-/state/com.google/read");
    }
    json!({
        "id": format!("tag:google.com,2005:reader/item/{n:016x}"),
        "title": title,
        "published": 1_700_000_000 + n,
        "categories": cats,
        "alternate": [{"href": format!("{base}/{n}")}],
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
    assert!(matches!(
        engine.refresh().await,
        Err(EngineError::NotConfigured)
    ));

    let account = engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    assert_eq!(
        account.base_url,
        format!("{}/api/greader.php", server.uri())
    );
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
    assert_eq!(
        engine
            .sources()
            .unwrap()
            .iter()
            .find(|s| s.source.title == "Graphics Blog")
            .unwrap()
            .unread,
        2
    );

    let doc = engine
        .document(&old.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
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
async fn read_history_backfills_in_the_background_and_resumes() {
    let server = mock_freshrss().await;
    let contents = "^/api/greader.php/reader/api/0/stream/contents/.*";
    let item = |n: u64, title: &str| item_at("https://blog.example", n, title, true);
    let page = |items: Vec<serde_json::Value>, next: Option<&str>| {
        ResponseTemplate::new(200).set_body_json(json!({"items": items, "continuation": next}))
    };
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param_is_missing("c"))
        .respond_with(page(
            vec![
                item_at("https://blog.example", 1, "Old post", true),
                item_at("https://blog.example", 2, "New post", false),
            ],
            Some("p2"),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param("c", "p2"))
        .respond_with(page(vec![item(10, "Archived one")], Some("p3")))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param("c", "p3"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param("c", "p3"))
        .respond_with(page(vec![item(11, "Archived two")], None))
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    let titles = |engine: &Engine| -> Vec<String> {
        engine
            .items(&ItemQuery::default())
            .unwrap()
            .into_iter()
            .map(|r| r.item.title)
            .collect()
    };

    // Nothing to walk before the first sync has found where history starts.
    assert_eq!(engine.backfill_history(None).await.unwrap(), 0);

    // A refresh takes only the newest page; history stays out of its way.
    let summary = engine.refresh().await.unwrap();
    assert_eq!(summary.new_items, 3);
    assert!(!titles(&engine).contains(&"Archived one".to_string()));

    // The walk reports progress per page and stops at a failing one; a
    // second walk started meanwhile returns at once.
    let totals = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen = totals.clone();
    let on_page: HistoryCallback = Arc::new(move |added| seen.lock().push(added));
    let (walk, overlapping) = tokio::join!(
        engine.backfill_history(Some(on_page)),
        engine.backfill_history(None)
    );
    assert!(walk.is_err());
    assert_eq!(overlapping.unwrap(), 0);
    assert_eq!(*totals.lock(), [1]);
    assert!(!titles(&engine).contains(&"Archived two".to_string()));
    let archived = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .find(|r| r.item.title == "Archived one")
        .unwrap();
    assert!(archived.item.read);
    assert_eq!(archived.source_title.as_deref(), Some("Graphics Blog"));

    // Refreshing keeps the saved position; the next walk resumes there and
    // reaches the end.
    assert_eq!(engine.refresh().await.unwrap().new_items, 0);
    assert_eq!(engine.backfill_history(None).await.unwrap(), 1);
    assert!(titles(&engine).contains(&"Archived two".to_string()));

    // Once complete, older pages are not requested again.
    assert_eq!(engine.backfill_history(None).await.unwrap(), 0);
    engine.refresh().await.unwrap();
}

#[tokio::test]
async fn local_changes_are_pushed_to_freshrss() {
    let server = mock_freshrss().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/edit-tag"))
        .and(body_string_contains(
            "a=user%2F-%2Fstate%2Fcom.google%2Fread",
        ))
        .and(body_string_contains("0000000000000002"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .expect(1..)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let new_post = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .find(|r| r.item.title == "New post")
        .unwrap();
    engine.set_read(&new_post.item.id, true).await.unwrap();
    assert!(engine.item(&new_post.item.id).unwrap().unwrap().item.read);
    // Refresh pushes pending changes before pulling.
    engine.refresh().await.unwrap();
    assert!(engine.inner.store.upstream_pending().unwrap().is_empty());
}

#[tokio::test]
async fn mark_all_read_takes_unread_subscription_articles_up_to_a_moment() {
    let server = mock_freshrss().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/edit-tag"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&server)
        .await;
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let saved = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    engine.set_read(&saved.item.id, false).await.unwrap();
    let unread_articles = |e: &Engine| {
        e.items(&ItemQuery {
            unread_only: true,
            ..Default::default()
        })
        .unwrap()
        .iter()
        .filter(|r| r.item.kind == eddy_model::ItemKind::Feed)
        .count()
    };
    let before = unread_articles(&engine);
    assert!(before > 0);

    // Nothing had arrived by the start of time.
    assert_eq!(engine.unread_feed_count(&ItemFilter::All, 0).unwrap(), 0);
    assert_eq!(engine.mark_all_read(&ItemFilter::All, 0).unwrap(), 0);

    let now = i64::MAX;
    assert_eq!(
        engine.unread_feed_count(&ItemFilter::All, now).unwrap() as usize,
        before
    );
    assert_eq!(
        engine.mark_all_read(&ItemFilter::All, now).unwrap() as usize,
        before
    );
    assert_eq!(unread_articles(&engine), 0);
    // Saved pages aren't subscription articles.
    assert!(!engine.item(&saved.item.id).unwrap().unwrap().item.read);
    // The marks reach FreshRSS with the next refresh.
    engine.refresh().await.unwrap();
    assert!(engine.inner.store.upstream_pending().unwrap().is_empty());
}

#[tokio::test]
async fn unpushed_local_changes_survive_a_refresh() {
    // No edit-tag endpoint: every push fails.
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let new_post = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .find(|r| r.item.title == "New post")
        .unwrap();
    engine.set_read(&new_post.item.id, true).await.unwrap();
    assert!(engine.refresh().await.is_err());
    assert!(engine.item(&new_post.item.id).unwrap().unwrap().item.read);
    assert_eq!(engine.inner.store.upstream_pending().unwrap().len(), 1);
    assert!(engine.status().unwrap().last_error.is_some());
}

fn article_html(title: &str, paragraphs: usize) -> String {
    let body: String = (0..paragraphs)
        .map(|i| format!("<p>Paragraph {i} of {title} explains importance sampling and why the estimator stays unbiased.</p>"))
        .collect();
    format!(
        "<html><head><title>{title} | Site</title></head><body><nav><a href='/'>Home</a></nav><article><h1>{title}</h1>{body}</article><footer>Footer</footer></body></html>"
    )
}

async fn mock_pages() -> MockServer {
    let server = MockServer::start().await;
    for n in 1..=3 {
        Mock::given(method("GET"))
            .and(path(format!("/{n}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(article_html(&format!("Full {n}"), 30)),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/essay"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(article_html("An Essay", 20)),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn full_text_replaces_a_truncated_feed_version() {
    let pages = mock_pages().await;
    let server = mock_freshrss_linking(&pages.uri()).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let row = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .find(|r| r.item.title == "New post")
        .unwrap();
    let before = engine.document(&row.item.id).await.unwrap().unwrap();

    let full = engine.fetch_full_text(&row.item.id).await.unwrap();
    assert_eq!(full.version.origin, eddy_model::ContentOrigin::Extracted);
    assert!(full.document.text_len() > before.document.text_len() * 3);
    assert!(!full.document.plain_text().contains("Footer"));

    let current = engine.document(&row.item.id).await.unwrap().unwrap();
    assert_eq!(current.version.hash, full.version.hash);
    let source = engine
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.source.title == "Graphics Blog")
        .unwrap();
    assert!(source.source.truncated);
    // Switching versions is an op, so it syncs like any other change.
    assert!(
        engine
            .inner
            .store
            .unuploaded_ops(100)
            .unwrap()
            .iter()
            .any(|o| o.op.field == "version")
    );
}

#[tokio::test]
async fn full_text_switches_off_and_back_without_refetching_and_stays() {
    use eddy_model::ContentOrigin;
    let pages = mock_pages().await;
    let server = mock_freshrss_linking(&pages.uri()).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .set_full_text_rules(vec![eddy_model::FullTextRule {
            scope: eddy_model::FullTextScope::All,
            mode: eddy_model::FullTextMode::Never,
        }])
        .unwrap();
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let id = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .find(|r| r.item.title == "New post")
        .unwrap()
        .item
        .id;
    // Off while it is already the feed's content: nothing changes.
    let feed = engine.set_full_text(&id, false).await.unwrap();
    assert_eq!(feed.version.origin, ContentOrigin::Feed);
    let page_requests = || async { pages.received_requests().await.unwrap().len() };
    assert_eq!(page_requests().await, 0);

    // On the first time fetches the page.
    let full = engine.set_full_text(&id, true).await.unwrap();
    assert_eq!(full.version.origin, ContentOrigin::Extracted);
    let fetched = page_requests().await;
    assert!(fetched > 0);

    // Off and on again switch between the kept versions.
    let back = engine.set_full_text(&id, false).await.unwrap();
    assert_eq!(back.version.hash, feed.version.hash);
    assert_eq!(
        engine.document(&id).await.unwrap().unwrap().version.origin,
        ContentOrigin::Feed
    );
    let again = engine.set_full_text(&id, true).await.unwrap();
    assert_eq!(again.version.hash, full.version.hash);
    assert_eq!(page_requests().await, fetched);

    // The choice stays, including across a refresh and reopening.
    engine.set_full_text(&id, false).await.unwrap();
    engine.refresh().await.unwrap();
    drop(engine);
    let engine = open(dir.path());
    assert_eq!(
        engine.document(&id).await.unwrap().unwrap().version.origin,
        ContentOrigin::Feed
    );
}

#[tokio::test]
async fn full_text_rules_pick_the_mode_with_the_top_rule_winning() {
    use eddy_model::{FullTextMode as M, FullTextRule, FullTextScope as S};
    let pages = mock_pages().await;
    let server = mock_freshrss_linking(&pages.uri()).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    // Before anything arrives: nothing in the "Tech" folder is fetched.
    engine
        .set_full_text_rules(vec![
            FullTextRule {
                scope: S::Category { name: "Tech".into() },
                mode: M::Never,
            },
            FullTextRule {
                scope: S::All,
                mode: M::Always,
            },
        ])
        .unwrap();
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let source = engine
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.source.title == "Graphics Blog")
        .unwrap()
        .source;
    let ids: Vec<String> = engine
        .items(&ItemQuery::default())
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect();
    assert_eq!(engine.effective_full_text(&source.id).unwrap(), M::Never);
    assert!(engine.inner.auto_full_text(&ids).is_empty());

    // Choosing a mode for the subscription puts its rule on top.
    engine
        .set_source_full_text_rule(&source.id, Some(M::Auto))
        .unwrap();
    engine
        .set_source_full_text_rule(&source.id, Some(M::Always))
        .unwrap();
    let rules = engine.full_text_rules().unwrap();
    assert_eq!(rules.len(), 3);
    assert_eq!(
        rules[0],
        FullTextRule {
            scope: S::Source {
                id: source.id.clone()
            },
            mode: M::Always
        }
    );
    assert_eq!(engine.effective_full_text(&source.id).unwrap(), M::Always);
    assert_eq!(engine.inner.auto_full_text(&ids).len(), ids.len());

    // Dropping it leaves the folder rule in charge again.
    engine.set_source_full_text_rule(&source.id, None).unwrap();
    assert_eq!(engine.full_text_rules().unwrap().len(), 2);
    assert_eq!(engine.effective_full_text(&source.id).unwrap(), M::Never);

    // Rules are a synced setting: another device applying the ops agrees.
    engine
        .set_source_full_text_rule(&source.id, Some(M::Always))
        .unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let other = open(other_dir.path());
    other
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    other.refresh().await.unwrap();
    assert_eq!(other.effective_full_text(&source.id).unwrap(), M::Auto);
    for op in engine.inner.store.unuploaded_ops(1000).unwrap() {
        if op.op.entity == eddy_model::Entity::Setting {
            other.inner.store.apply_remote(&op.op).unwrap();
        }
    }
    assert_eq!(other.full_text_rules().unwrap(), engine.full_text_rules().unwrap());
    assert_eq!(other.effective_full_text(&source.id).unwrap(), M::Always);
}

#[tokio::test]
async fn walls_and_short_pages_never_replace_feed_content() {
    let pages = MockServer::start().await;
    let html = |body: &str| {
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/html")
            .set_body_string(format!("<html><body>{body}</body></html>"))
    };
    Mock::given(method("GET"))
        .and(path("/1"))
        .respond_with(html(
            "<div class='consent'><p>We use cookies to enhance your browsing experience and analyse our traffic. By clicking accept you consent.</p></div>",
        ))
        .mount(&pages)
        .await;
    Mock::given(method("GET"))
        .and(path("/2"))
        .respond_with(html(
            "<p>Sign up for the most real communities. Continue with phone. Continue with email.</p>\
             <ac-track san=\"xpromo|click|blocking_xpromo_nsfw_blocking\"></ac-track>",
        ))
        .mount(&pages)
        .await;
    let server = mock_freshrss_linking(&pages.uri()).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let id_of = |title: &str| {
        engine
            .items(&ItemQuery::default())
            .unwrap()
            .into_iter()
            .find(|r| r.item.title == title)
            .unwrap()
            .item
            .id
    };
    let origin_of = |engine: &Engine, id: &str| {
        let row = engine.inner.store.item(id).unwrap().unwrap();
        engine
            .inner
            .store
            .content_version(row.item.current_version.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .origin
    };
    let (old, new) = (id_of("Old post"), id_of("New post"));

    // A sign-in prompt is refused even when asked for.
    assert!(matches!(
        engine.fetch_full_text(&new).await,
        Err(EngineError::Invalid(m)) if m.contains("登录")
    ));
    assert_eq!(origin_of(&engine, &new), eddy_model::ContentOrigin::Feed);

    // In the background, a page that adds little keeps the feed version.
    assert!(engine.inner.fetch_full_text(&old, true).await.is_err());
    assert_eq!(origin_of(&engine, &old), eddy_model::ContentOrigin::Feed);

    // Pages that replaced short feed content before go back to the feed
    // version once, on opening.
    engine.fetch_full_text(&old).await.unwrap();
    assert_eq!(
        origin_of(&engine, &old),
        eddy_model::ContentOrigin::Extracted
    );
    engine
        .inner
        .store
        .meta_delete("restored_feed_versions")
        .unwrap();
    drop(engine);
    let engine = open(dir.path());
    assert_eq!(origin_of(&engine, &old), eddy_model::ContentOrigin::Feed);
    assert!(
        engine
            .inner
            .store
            .meta_get("restored_feed_versions")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_gallery_holds_marked_sources_folders_and_saved_media() {
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let all = || engine.gallery_count(&GalleryScope::All, false).unwrap();
    assert_eq!(all(), 0);

    engine.set_folder_gallery("Tech", true).unwrap();
    assert_eq!(engine.gallery_folders().unwrap(), ["Tech"]);
    assert_eq!(all(), 3);
    assert_eq!(
        engine
            .gallery_count(&GalleryScope::Folder("Tech".into()), false)
            .unwrap(),
        3
    );
    engine.set_folder_gallery("Tech", false).unwrap();
    assert_eq!(all(), 0);

    let source = engine
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.source.title == "Graphics Blog")
        .unwrap()
        .source;
    engine.set_source_gallery(&source.id, true).unwrap();
    assert_eq!(all(), 3);
    let entries = engine
        .gallery_items(&GalleryScope::Source(source.id.clone()), false, 10, 0)
        .unwrap();
    assert_eq!(entries[0].row.item.title, "Unread elsewhere");
    assert!(engine.in_gallery(&entries[0].row).unwrap());

    // A link that is no media and a feed body without pictures: once
    // resolved, the item leaves the Gallery.
    let text_only = entries
        .iter()
        .find(|e| e.row.item.title == "Unread elsewhere")
        .unwrap();
    let set = engine
        .resolve_media(&text_only.row.item.id, false)
        .await
        .unwrap();
    assert_eq!(set.status, MediaStatus::NoMedia);
    assert_eq!(all(), 2);

    // Saved media links are the collection's Media part and in the Gallery.
    let saved = engine
        .inner
        .create_saved(
            "https://youtu.be/abcdefghijk",
            &article_html("A Talk", 5),
            eddy_model::ContentOrigin::Captured,
            None,
        )
        .unwrap();
    let entry = engine
        .collection()
        .unwrap()
        .into_iter()
        .find(|e| e.id == saved.item.id)
        .unwrap();
    assert_eq!(entry.kind, CollectionKind::Media);
    let saved_entries = engine
        .gallery_items(&GalleryScope::Saved, false, 10, 0)
        .unwrap();
    assert_eq!(saved_entries.len(), 1);
    assert!(saved_entries[0].media.is_none());
    let set = engine.resolve_media(&saved.item.id, false).await.unwrap();
    assert_eq!(set.items[0].kind, MediaType::Embed);
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg")
    );
    let again = engine
        .gallery_items(&GalleryScope::Saved, false, 10, 0)
        .unwrap();
    assert_eq!(again[0].media.as_ref(), Some(&set));
    assert_eq!(all(), 3);
}

#[tokio::test]
async fn removed_media_is_dropped_and_empty_items_leave_the_gallery() {
    let server = mock_freshrss().await;
    let files = MockServer::start().await;
    for (p, status) in [("/kept.jpg", 200), ("/gone.jpg", 404), ("/thumb.jpg", 410)] {
        Mock::given(path(p))
            .respond_with(ResponseTemplate::new(status))
            .mount(&files)
            .await;
    }
    // Imgur-style: a removed file redirects to a stand-in picture.
    Mock::given(path("/moved.jpg"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/removed.png", files.uri())),
        )
        .mount(&files)
        .await;
    Mock::given(path("/removed.png"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&files)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    engine.set_folder_gallery("Tech", true).unwrap();
    let entries = engine
        .gallery_items(&GalleryScope::All, false, 10, 0)
        .unwrap();
    let (a, b) = (&entries[0].row.item.id, &entries[1].row.item.id);
    let url = |p: &str| format!("{}{p}", files.uri());
    let store = |id: &str, set: &MediaSet| {
        engine
            .inner
            .store
            .put_media_set(id, &serde_json::to_string(set).unwrap(), "ready")
            .unwrap();
    };
    store(
        a,
        &MediaSet {
            status: MediaStatus::Ready,
            items: vec![
                eddy_media::MediaItem::image(url("/gone.jpg")),
                eddy_media::MediaItem::image(url("/kept.jpg")),
                // No answer at all is not "removed".
                eddy_media::MediaItem::image("http://127.0.0.1:9/unreachable.jpg"),
            ],
            thumbnail: Some(url("/thumb.jpg")),
        },
    );
    store(
        b,
        &MediaSet {
            status: MediaStatus::Ready,
            items: vec![
                eddy_media::MediaItem::image(url("/gone.jpg")),
                eddy_media::MediaItem::image(url("/moved.jpg")),
            ],
            thumbnail: Some(url("/gone.jpg")),
        },
    );
    let count = || engine.gallery_count(&GalleryScope::All, false).unwrap();
    assert_eq!(count(), 3);

    let a_set = engine.verify_media(a).await.unwrap().unwrap();
    assert_eq!(a_set.status, MediaStatus::Ready);
    let srcs: Vec<&str> = a_set.items.iter().map(|m| m.src.as_str()).collect();
    assert_eq!(srcs, [url("/kept.jpg").as_str(), "http://127.0.0.1:9/unreachable.jpg"]);
    assert_eq!(a_set.thumbnail, Some(url("/kept.jpg")));

    let b_set = engine.verify_media(b).await.unwrap().unwrap();
    assert_eq!(b_set.status, MediaStatus::NoMedia);
    assert!(b_set.items.is_empty() && b_set.thumbnail.is_none());
    assert_eq!(count(), 2);
    // Kept across reopening, like any resolved set.
    assert_eq!(
        engine.resolve_media(b, false).await.unwrap().status,
        MediaStatus::NoMedia
    );
}

#[tokio::test]
async fn saving_a_media_link_keeps_the_media_instead_of_a_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "video/mp4"))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let url = format!("{}/clip.mp4", server.uri());
    let row = engine.save_url(&url).await.unwrap();
    assert_eq!(row.item.title, "127.0.0.1 · clip.mp4");
    let doc = engine
        .document(&row.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    assert!(doc.blocks.iter().any(|b| matches!(
        &b.kind,
        BlockKind::Media { media: eddy_model::MediaKind::Video, src, .. } if src == &url
    )));
    let entry = engine
        .collection()
        .unwrap()
        .into_iter()
        .find(|e| e.id == row.item.id)
        .unwrap();
    assert_eq!(entry.kind, CollectionKind::Media);
    let entries = engine
        .gallery_items(&GalleryScope::Saved, false, 10, 0)
        .unwrap();
    assert_eq!(entries[0].media.as_ref().unwrap().items[0].src, url);
    // Saving it again finds the same item.
    assert_eq!(engine.save_url(&url).await.unwrap().item.id, row.item.id);
}

#[tokio::test]
async fn saving_urls_and_importing_captures() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());

    let saved = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    assert_eq!(saved.item.title, "An Essay");
    assert_eq!(saved.item.kind, eddy_model::ItemKind::Saved);
    let again = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    assert_eq!(again.item.id, saved.item.id);
    let doc = engine.document(&saved.item.id).await.unwrap().unwrap();
    assert!(doc.document.text_len() > 500);

    // A capture written by the browser host, with logged-in content.
    let spool = engine.spool_dir();
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(
        spool.join("0001.json"),
        serde_json::to_vec(&json!({"url": "https://paper.example/members/1", "title": "Members only", "html": article_html("Members only", 15)})).unwrap(),
    )
    .unwrap();
    std::fs::write(spool.join("0002.json"), b"not json").unwrap();
    assert_eq!(engine.import_captures().await.unwrap(), 1);
    assert!(spool.join("failed/0002.json").exists());
    let saved_items = engine
        .items(&ItemQuery {
            filter: ItemFilter::Saved,
            ..Default::default()
        })
        .unwrap();
    let captured = saved_items
        .iter()
        .find(|r| r.item.title == "Members only")
        .unwrap();
    let doc = engine.document(&captured.item.id).await.unwrap().unwrap();
    assert_eq!(doc.version.origin, eddy_model::ContentOrigin::Captured);
    // The extension's "Show" finds it by the address it captured.
    assert_eq!(
        engine
            .saved_item_for_url("https://paper.example/members/1")
            .unwrap()
            .as_deref(),
        Some(captured.item.id.as_str())
    );
    assert_eq!(
        engine
            .saved_item_for_url(&format!("{}/essay", pages.uri()))
            .unwrap(),
        Some(saved.item.id.clone())
    );
    assert_eq!(
        engine
            .saved_item_for_url("https://paper.example/other")
            .unwrap(),
        None
    );

    // Evicting the derived document regenerates it by re-extracting the page.
    let blob = doc.version.doc_blob.clone().unwrap();
    std::fs::remove_file(
        dir.path()
            .join("blobs")
            .join(&blob[blob.len() - 2..])
            .join(&blob),
    )
    .unwrap();
    let regenerated = engine.document(&captured.item.id).await.unwrap().unwrap();
    assert_eq!(regenerated.document, doc.document);
}

#[tokio::test]
async fn private_rule_packs_load_from_the_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    assert!(engine.rule_packs().packs.is_empty());
    let summary = engine
        .import_rule_pack("mine", "name = \"Mine\"\nversion = 1\n[[site]]\ndomains = [\"example.com\"]\nstrategies = [\"archive\"]\n")
        .unwrap();
    assert_eq!(summary.packs, vec![("Mine".to_string(), 1)]);
    assert!(engine.import_rule_pack("bad", "[[site]\n").is_err());
    assert!(
        engine
            .import_rule_pack("../escape", "name = \"x\"")
            .is_err()
    );
    let summary = engine
        .import_ftr_rule(".example.org", "body: //article\n")
        .unwrap();
    assert_eq!(summary.ftr_sites, 1);
    assert_eq!(
        engine
            .inner
            .fetcher
            .read()
            .rule_for("https://www.example.com/a")
            .unwrap()
            .strategies,
        ["archive"]
    );
    assert!(
        engine
            .inner
            .fetcher
            .read()
            .rule_for("https://news.example.org/a")
            .is_some()
    );
}

async fn mock_llm() -> MockServer {
    let server = MockServer::start().await;
    let reply = |content: serde_json::Value| {
        ResponseTemplate::new(200)
            .set_body_json(json!({"choices": [{"message": {"content": content.to_string()}}]}))
    };
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("You summarize articles"))
        .respond_with(reply(json!({"tldr": "一句话", "points": ["要点"]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("reader's own tags"))
        .respond_with(reply(
            json!({"tags": ["Rendering", "Rust"], "suggestions": ["Sampling"]}),
        ))
        .mount(&server)
        .await;
    // Translation: echo every segment back with a prefix.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("Translate each input segment"))
        .respond_with(|req: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(
                json!({"choices": [{"message": {"content": echo_translation(&requested_segments(req))}}]}),
            )
        })
        .mount(&server)
        .await;
    server
}

/// The segments a translation request carries, `(id, text)`, read from
/// its marker lines.
fn requested_segments(req: &wiremock::Request) -> Vec<(String, String)> {
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    let user = body["messages"][1]["content"].as_str().unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in user.lines() {
        if let Some(id) = line.strip_prefix("<<<").and_then(|r| r.strip_suffix(">>>")) {
            out.push((id.to_string(), String::new()));
        } else if let Some(last) = out.last_mut() {
            last.1.push_str(line);
        }
    }
    out
}

/// A model reply translating each segment as "译：" plus the source.
fn echo_translation(segments: &[(String, String)]) -> String {
    segments
        .iter()
        .map(|(id, text)| format!("<<<{id}>>>\n译：{text}\n"))
        .collect()
}

fn configure_ai(engine: &Engine, llm: &MockServer, rules: Vec<AutoRule>) {
    configure_ai_with(engine, llm, rules, 1);
}

fn configure_ai_with(engine: &Engine, llm: &MockServer, rules: Vec<AutoRule>, concurrency: u32) {
    engine
        .set_ai_settings(AiSettings {
            provider: Some(ProviderSettings {
                base_url: llm.uri(),
                model: "test-model".into(),
                max_concurrency: concurrency,
            }),
            rules,
            ..Default::default()
        })
        .unwrap();
    engine.set_ai_key(Some("sk-test-secret-key")).unwrap();
}

#[tokio::test]
async fn cancelled_translation_keeps_what_is_done_and_resumes() {
    let pages = mock_pages().await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(&engine, &llm, vec![]);
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let id = row.item.id.clone();

    // Cancelled while the first batch is being handed over.
    let stopper = engine.clone();
    let stop_id = id.clone();
    let cancel: BatchCallback = Arc::new(move |_| stopper.cancel_translation(&stop_id));
    let partial = engine.translate(&id, false, Some(cancel)).await.unwrap();
    assert!(!partial.complete);
    assert!(!partial.segments.is_empty());
    assert_eq!(llm.received_requests().await.unwrap().len(), 1);

    let cached = engine.cached_translation(&id).unwrap().unwrap();
    assert!(!cached.complete);
    assert_eq!(cached.segments, partial.segments);

    // Resuming asks only for the batches still missing.
    let batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = batches.clone();
    let count: BatchCallback = Arc::new(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let full = engine.translate(&id, false, Some(count)).await.unwrap();
    assert!(full.complete);
    let total = batches.load(std::sync::atomic::Ordering::SeqCst);
    assert!(total >= 3);
    assert_eq!(llm.received_requests().await.unwrap().len(), total);
    assert!(full.segments.len() > partial.segments.len());
    assert!(engine.cached_translation(&id).unwrap().unwrap().complete);
}

/// A translating model that answers after `delay`, recording the segment
/// IDs of each request in arrival order and the most requests it has seen
/// unanswered at once (requests arrived minus batches handed over).
#[derive(Clone, Default)]
struct SlowTranslator {
    delay_ms: u64,
    arrivals: Arc<parking_lot::Mutex<Vec<Vec<String>>>>,
    delivered: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
}

impl wiremock::Respond for SlowTranslator {
    fn respond(&self, req: &wiremock::Request) -> ResponseTemplate {
        let segments = requested_segments(req);
        let mut arrivals = self.arrivals.lock();
        arrivals.push(segments.iter().map(|(id, _)| id.clone()).collect());
        let open = arrivals.len() - self.delivered.load(std::sync::atomic::Ordering::SeqCst);
        self.peak
            .fetch_max(open, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_millis(self.delay_ms))
            .set_body_json(json!({"choices": [{"message": {"content": echo_translation(&segments)}}]}))
    }
}

impl SlowTranslator {
    async fn serve(delay_ms: u64) -> (MockServer, SlowTranslator) {
        let server = MockServer::start().await;
        let model = SlowTranslator {
            delay_ms,
            ..Default::default()
        };
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(model.clone())
            .mount(&server)
            .await;
        (server, model)
    }

    fn on_batch(&self) -> BatchCallback {
        let delivered = self.delivered.clone();
        Arc::new(move |_| {
            delivered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }
}

async fn essay_paragraphs(engine: &Engine, id: &str) -> Vec<String> {
    engine
        .document(id)
        .await
        .unwrap()
        .unwrap()
        .document
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Paragraph { .. }))
        .map(|b| b.id.clone())
        .collect()
}

#[tokio::test]
async fn translation_starts_where_the_reader_is_and_follows_scrolling() {
    let pages = mock_pages().await;
    let (llm, model) = SlowTranslator::serve(0).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(&engine, &llm, vec![]);
    let id = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap()
        .item
        .id;
    let paragraphs = essay_paragraphs(&engine, &id).await;
    let last = paragraphs.last().unwrap().clone();
    let first = paragraphs[0].clone();

    // Reading the end: its batch goes first. After it, the reader scrolls
    // back to the top, which goes next.
    engine.set_translation_focus(&id, &last);
    let mover = engine.clone();
    let (move_id, move_to) = (id.clone(), first.clone());
    let delivered = model.on_batch();
    let callback: BatchCallback = Arc::new(move |batch| {
        delivered(batch);
        mover.set_translation_focus(&move_id, &move_to);
    });
    let t = engine.translate(&id, false, Some(callback)).await.unwrap();
    assert!(t.complete);
    let arrivals = model.arrivals.lock().clone();
    assert!(arrivals.len() >= 3);
    assert!(arrivals[0].contains(&last));
    assert!(arrivals[1].contains(&first) && arrivals[1].contains(&"title".to_string()));
    // Every batch was asked for once.
    let asked: usize = arrivals.iter().map(Vec::len).sum();
    assert_eq!(asked, t.segments.len());
}

#[tokio::test]
async fn translation_runs_batches_concurrently_within_the_limit() {
    let pages = mock_pages().await;
    let (llm, model) = SlowTranslator::serve(150).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai_with(&engine, &llm, vec![], 2);
    let id = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap()
        .item
        .id;
    let t = engine
        .translate(&id, false, Some(model.on_batch()))
        .await
        .unwrap();
    assert!(t.complete);
    assert!(model.arrivals.lock().len() >= 3);
    assert_eq!(model.peak.load(std::sync::atomic::Ordering::SeqCst), 2);

    // Done batches are reused, whatever order they were done in.
    let again = engine.translate(&id, true, None).await.unwrap();
    assert_eq!(again.segments, t.segments);
    let before = model.arrivals.lock().len();
    engine.translate(&id, false, None).await.unwrap();
    assert_eq!(model.arrivals.lock().len(), before);

    // Settings never go above the cap.
    configure_ai_with(&engine, &llm, vec![], 50);
    assert_eq!(
        engine
            .ai_settings()
            .unwrap()
            .provider
            .unwrap()
            .max_concurrency,
        crate::MAX_CONCURRENCY
    );
}

#[tokio::test]
async fn cancelling_with_requests_in_flight_keeps_what_they_return() {
    let pages = mock_pages().await;
    let (llm, model) = SlowTranslator::serve(150).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai_with(&engine, &llm, vec![], 2);
    let id = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap()
        .item
        .id;
    let stopper = engine.clone();
    let stop_id = id.clone();
    let delivered = model.on_batch();
    let cancel: BatchCallback = Arc::new(move |batch| {
        delivered(batch);
        stopper.cancel_translation(&stop_id);
    });
    let partial = engine.translate(&id, false, Some(cancel)).await.unwrap();
    assert!(!partial.complete);
    // Both requests sent before the cancel came back and were kept.
    let arrivals = model.arrivals.lock().clone();
    assert_eq!(arrivals.len(), 2);
    let asked: usize = arrivals.iter().map(Vec::len).sum();
    assert_eq!(partial.segments.len(), asked);
    assert_eq!(
        engine.cached_translation(&id).unwrap().unwrap().segments,
        partial.segments
    );
}

#[tokio::test]
async fn provider_test_sends_the_configured_requests_at_once() {
    let (llm, model) = SlowTranslator::serve(150).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let provider = ProviderSettings {
        base_url: llm.uri(),
        model: "test-model".into(),
        max_concurrency: 3,
    };
    let r = engine
        .test_ai_provider(provider.clone(), Some("sk-test"), "简体中文")
        .await
        .unwrap();
    assert_eq!((r.total, r.passed, r.error), (3, 3, None));
    assert_eq!(model.peak.load(std::sync::atomic::Ordering::SeqCst), 3);
    // Without a key given or saved, it says so.
    assert!(
        engine
            .test_ai_provider(provider, None, "简体中文")
            .await
            .is_err()
    );

    // A rate-limited service fails the test with the reason.
    let limited = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
        .mount(&limited)
        .await;
    let r = engine
        .test_ai_provider(
            ProviderSettings {
                base_url: limited.uri(),
                model: "m".into(),
                max_concurrency: 2,
            },
            Some("sk-test"),
            "简体中文",
        )
        .await
        .unwrap();
    assert_eq!(r.passed, 0);
    assert!(r.error.unwrap().contains("429"));
}

#[tokio::test]
async fn translation_summary_and_tags_with_caching() {
    let pages = mock_pages().await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(&engine, &llm, vec![]);
    assert!(
        engine
            .ai_settings()
            .unwrap()
            .provider
            .unwrap()
            .base_url
            .ends_with("/v1")
    );
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();

    let batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = batches.clone();
    let callback: BatchCallback = Arc::new(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let t = engine
        .translate(&row.item.id, false, Some(callback))
        .await
        .unwrap();
    assert!(!t.cached);
    assert_eq!(t.segments["title"], "译：An Essay");
    let doc = engine
        .document(&row.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    let first_para = doc
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Paragraph { .. }))
        .unwrap();
    assert!(t.segments[&first_para.id].starts_with("译：Paragraph 0"));
    assert!(batches.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    let requests_after_first = llm.received_requests().await.unwrap().len();

    let again = engine.translate(&row.item.id, false, None).await.unwrap();
    assert!(again.cached);
    assert_eq!(
        llm.received_requests().await.unwrap().len(),
        requests_after_first
    );
    assert_eq!(
        engine
            .cached_translation(&row.item.id)
            .unwrap()
            .unwrap()
            .segments,
        t.segments
    );

    let s = engine.summarize(&row.item.id, false).await.unwrap();
    assert_eq!(s.summary.tldr, "一句话");
    assert!(engine.summarize(&row.item.id, false).await.unwrap().cached);

    // A page a link opened is summarized from its HTML, and not stored.
    let paragraphs: String = (0..8)
        .map(|i| format!("<p>Paragraph {i} with enough words to read as an article body.</p>"))
        .collect();
    let page = format!(
        "<html><head><title>Linked</title></head><body><article><h1>Linked</h1>{paragraphs}</article></body></html>"
    );
    let items_before = engine.items(&ItemQuery::default()).unwrap().len();
    let linked = engine
        .summarize_page("https://example.com/linked", &page)
        .await
        .unwrap();
    assert_eq!(linked.tldr, "一句话");
    assert_eq!(
        engine.items(&ItemQuery::default()).unwrap().len(),
        items_before
    );

    let rendering = engine.create_tag("Rendering").unwrap();
    assert_eq!(engine.create_tag(" rendering ").unwrap(), rendering);
    let tags = engine.suggest_tags(&row.item.id).await.unwrap();
    assert_eq!(
        tags.tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Rendering"]
    );
    assert_eq!(tags.suggestions, ["Rust", "Sampling"]);
    engine
        .accept_tag_suggestion(&row.item.id, "Sampling")
        .unwrap();
    engine
        .set_item_tag(&row.item.id, &rendering, false)
        .unwrap();
    let after = engine.suggest_tags(&row.item.id).await.unwrap();
    assert_eq!(
        after
            .tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Sampling"]
    );
    assert_eq!(
        engine
            .items(&ItemQuery {
                filter: ItemFilter::Tag(eddy_model::ids::tag_id("sampling")),
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );

    // The key lives in the keychain, never in the database.
    let db = std::fs::read(dir.path().join("eddy.sqlite3")).unwrap();
    let wal = std::fs::read(dir.path().join("eddy.sqlite3-wal")).unwrap_or_default();
    let needle = b"sk-test-secret-key";
    assert!(
        !db.windows(needle.len()).any(|w| w == needle)
            && !wal.windows(needle.len()).any(|w| w == needle)
    );
}

#[tokio::test]
async fn automatic_rules_run_on_new_items() {
    let pages = mock_pages().await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(
        &engine,
        &llm,
        vec![AutoRule {
            scope: RuleScope::All,
            action: AutoAction::Summarize,
            min_chars: 100,
            only_foreign: true,
        }],
    );
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let mut summary = None;
    for _ in 0..50 {
        summary = engine.cached_summary(&row.item.id).unwrap();
        if summary.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(summary.unwrap().tldr, "一句话");
    assert_eq!(engine.ai_usage_today().unwrap(), 1);
}

#[tokio::test]
async fn rules_added_later_apply_to_items_as_they_are_opened() {
    use eddy_model::{ContentOrigin, FullTextMode as M, FullTextRule, FullTextScope as S};
    let pages = mock_pages().await;
    let server = mock_freshrss_linking(&pages.uri()).await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let rule = |engine: &Engine, mode| {
        engine
            .set_full_text_rules(vec![FullTextRule {
                scope: S::All,
                mode,
            }])
            .unwrap()
    };
    rule(&engine, M::Never);
    configure_ai(&engine, &llm, vec![]);
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let id_of = |title: &str| {
        engine
            .items(&ItemQuery::default())
            .unwrap()
            .into_iter()
            .find(|r| r.item.title == title)
            .unwrap()
            .item
            .id
    };
    let (new_post, old_post) = (id_of("New post"), id_of("Old post"));
    assert!(!engine.full_text_on_open(&new_post).unwrap());
    let nothing = engine.apply_rules(&new_post).await.unwrap();
    assert!(nothing.document.is_none() && !nothing.summarize);

    // The rules change after the items arrived.
    rule(&engine, M::Always);
    configure_ai(
        &engine,
        &llm,
        vec![
            AutoRule {
                scope: RuleScope::All,
                action: AutoAction::Summarize,
                min_chars: 0,
                only_foreign: false,
            },
            AutoRule {
                scope: RuleScope::All,
                action: AutoAction::Tag,
                min_chars: 0,
                only_foreign: false,
            },
        ],
    );
    engine.create_tag("Rust").unwrap();
    assert!(engine.full_text_on_open(&new_post).unwrap());
    let applied = engine.apply_rules(&new_post).await.unwrap();
    let full = applied.document.unwrap();
    assert_eq!(full.version.origin, ContentOrigin::Extracted);
    assert_eq!(
        engine.document(&new_post).await.unwrap().unwrap().version.hash,
        full.version.hash
    );
    // The summary is the reader's to show; tags come in the background.
    assert!(applied.summarize && !applied.translate);
    assert!(engine.cached_summary(&new_post).unwrap().is_none());
    for _ in 0..50 {
        if engine.inner.tagged(&new_post).unwrap() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(engine.item_tags(&new_post).unwrap().tags.len(), 1);
    engine.summarize(&new_post, false).await.unwrap();
    let summarized = engine
        .inner
        .store
        .latest_ai_artifact(&new_post, "summary")
        .unwrap()
        .unwrap();
    assert_eq!(summarized.0, full.version.hash);

    // Opening again: the summary still shows, from the cache; tagging
    // isn't repeated, the page isn't fetched again.
    let requests = llm.received_requests().await.unwrap().len();
    let fetched = pages.received_requests().await.unwrap().len();
    assert!(!engine.full_text_on_open(&new_post).unwrap());
    let again = engine.apply_rules(&new_post).await.unwrap();
    assert!(again.document.is_none() && again.summarize);
    assert!(engine.summarize(&new_post, false).await.unwrap().cached);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(llm.received_requests().await.unwrap().len(), requests);
    assert_eq!(pages.received_requests().await.unwrap().len(), fetched);

    // Switched back to the feed's version, it stays there.
    engine.set_full_text(&old_post, true).await.unwrap();
    engine.set_full_text(&old_post, false).await.unwrap();
    assert!(!engine.full_text_on_open(&old_post).unwrap());
    assert!(engine.apply_rules(&old_post).await.unwrap().document.is_none());
    assert_eq!(
        engine.document(&old_post).await.unwrap().unwrap().version.origin,
        ContentOrigin::Feed
    );
}

#[tokio::test]
async fn the_same_summary_asked_twice_at_once_is_one_request() {
    let pages = mock_pages().await;
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(150))
                .set_body_json(json!({"choices": [{"message": {"content":
                    json!({"tldr": "一句话", "points": []}).to_string()}}]})),
        )
        .mount(&llm)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai_with(&engine, &llm, vec![], 2);
    let id = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap()
        .item
        .id;
    let (a, b) = tokio::join!(engine.summarize(&id, false), engine.summarize(&id, false));
    assert!(!a.unwrap().cached);
    assert!(b.unwrap().cached);
    assert_eq!(llm.received_requests().await.unwrap().len(), 1);
}

fn para_ids(doc: &eddy_model::Document) -> Vec<String> {
    doc.blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Paragraph { .. }))
        .map(|b| b.id.clone())
        .collect()
}

#[tokio::test]
async fn progress_is_throttled_finishes_and_feeds_continue_reading() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let doc = engine
        .document(&row.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    let paras = para_ids(&doc);
    let ops = |e: &Engine| {
        e.inner
            .store
            .unuploaded_ops(10_000)
            .unwrap()
            .iter()
            .filter(|o| o.op.field == "locator")
            .count()
    };

    let mid = engine
        .record_progress(&row.item.id, &paras[paras.len() / 2], 3, false, false)
        .unwrap()
        .unwrap();
    assert!(
        mid.progression > 0.3 && mid.progression < 0.7,
        "{}",
        mid.progression
    );
    engine
        .record_progress(&row.item.id, &paras[paras.len() / 2 + 1], 0, false, false)
        .unwrap();
    assert_eq!(
        ops(&engine),
        1,
        "scrolling within the interval is throttled"
    );
    engine
        .record_progress(&row.item.id, &paras[paras.len() / 2 + 1], 0, false, true)
        .unwrap();
    assert_eq!(ops(&engine), 2);
    let shelf = engine
        .items(&ItemQuery {
            filter: ItemFilter::ContinueReading,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(shelf.len(), 1);

    let resume = engine.resume_point(&row.item.id).unwrap().unwrap();
    assert_eq!(
        resume.block_id.as_deref(),
        Some(paras[paras.len() / 2 + 1].as_str())
    );
    assert!(!resume.from_other_device);

    let last = doc.blocks.last().unwrap().id.clone();
    engine
        .record_progress(&row.item.id, &last, 0, false, true)
        .unwrap();
    assert!(engine.item(&row.item.id).unwrap().unwrap().item.finished);
    assert!(
        engine
            .items(&ItemQuery {
                filter: ItemFilter::ContinueReading,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn end_on_screen_finishes_without_waiting_for_the_last_block() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let doc = engine
        .document(&row.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    let paras = para_ids(&doc);
    let top = &paras[paras.len() / 2];
    engine
        .record_progress(&row.item.id, top, 0, false, false)
        .unwrap();
    assert!(!engine.item(&row.item.id).unwrap().unwrap().item.finished);

    // Within the throttle interval: finishing is never held back.
    let locator = engine
        .record_progress(&row.item.id, top, 0, true, false)
        .unwrap()
        .unwrap();
    assert!(locator.progression >= 0.98);
    assert!(engine.item(&row.item.id).unwrap().unwrap().item.finished);
    assert!(
        engine
            .items(&ItemQuery {
                filter: ItemFilter::ContinueReading,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    // Reading resumes where the text was, not at the end.
    let resume = engine.resume_point(&row.item.id).unwrap().unwrap();
    assert_eq!(resume.block_id.as_deref(), Some(top.as_str()));
}

#[tokio::test]
async fn mark_unread_resets_reading_but_the_top_of_the_text_is_still_read() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let row = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let id = row.item.id.clone();
    let doc = engine.document(&id).await.unwrap().unwrap().document;
    let paras = para_ids(&doc);
    let continue_reading = |e: &Engine| {
        e.items(&ItemQuery {
            filter: ItemFilter::ContinueReading,
            ..Default::default()
        })
        .unwrap()
        .len()
    };

    // Back at the very top: a position at 0 that still counts as read.
    engine
        .record_progress(&id, &doc.blocks[0].id, 0, false, true)
        .unwrap();
    assert!(
        engine
            .item(&id)
            .unwrap()
            .unwrap()
            .item
            .last_read_at
            .is_some()
    );

    engine.set_read(&id, true).await.unwrap();
    engine
        .record_progress(&id, &paras[paras.len() / 2], 0, false, true)
        .unwrap();
    engine.set_finished(&id, true).unwrap();
    engine.set_finished(&id, false).unwrap();
    assert_eq!(continue_reading(&engine), 1);

    engine.mark_unread(std::slice::from_ref(&id)).unwrap();
    let item = engine.item(&id).unwrap().unwrap().item;
    assert!(!item.read);
    assert!(!item.finished);
    assert_eq!(item.progress, 0.0);
    assert!(item.last_read_at.is_none());
    assert!(engine.resume_point(&id).unwrap().is_none());
    assert_eq!(continue_reading(&engine), 0);
}

#[tokio::test]
async fn resume_point_follows_the_switch_to_full_text() {
    let pages = MockServer::start().await;
    // Feed excerpt and full page share the second paragraph.
    let shared =
        "The shared paragraph describes multiple importance sampling in detail for the reader.";
    let body: String = (0..25)
        .map(|i| {
            format!("<p>Full paragraph {i} adds detail about light transport and variance.</p>")
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/2"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(format!(
            "<html><head><title>New post</title></head><body><article><p>Intro only in the full text.</p><p>{shared}</p>{body}</article></body></html>"
        )))
        .mount(&pages)
        .await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=t"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/subscription/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"subscriptions": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"itemRefs": [{"id": "2"}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/api/greader.php/reader/api/0/stream/contents/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{
            "id": "tag:google.com,2005:reader/item/0000000000000002", "title": "New post", "published": 1_700_000_000,
            "categories": [], "alternate": [{"href": format!("{}/2", pages.uri())}],
            "summary": {"content": format!("<p>Teaser line.</p><p>{shared}</p>")}, "origin": {"streamId": "feed/1"}
        }]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/stream/items/contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "pw")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let item = engine.items(&ItemQuery::default()).unwrap().remove(0);
    let feed_doc = engine
        .document(&item.item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    let shared_id = para_ids(&feed_doc)[1].clone();
    engine
        .record_progress(&item.item.id, &shared_id, 13, false, true)
        .unwrap();

    let full = engine.fetch_full_text(&item.item.id).await.unwrap();
    let resume = engine.resume_point(&item.item.id).unwrap().unwrap();
    assert_eq!(resume.locator.content_hash, full.version.hash);
    assert_eq!(
        resume.block_id.as_deref(),
        Some(shared_id.as_str()),
        "same paragraph keeps its ID across versions"
    );
    assert_eq!(resume.offset, 13);
}

#[tokio::test]
async fn series_from_a_table_of_contents() {
    let site = MockServer::start().await;
    let toc: String = (1..=4)
        .map(|i| format!("<li><a href=\"/book/ch{i}.html\">Chapter {i}</a></li>"))
        .collect();
    Mock::given(method("GET"))
        .and(path("/book/index.html"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(format!(
            "<html><head><title>Ray Tracing Book</title></head><body><nav class='toc'><ol>{toc}</ol></nav><main><p>Welcome.</p></main></body></html>"
        )))
        .mount(&site)
        .await;
    for i in 1..=4 {
        Mock::given(method("GET"))
            .and(path(format!("/book/ch{i}.html")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(article_html(&format!("Chapter {i}"), 12)),
            )
            .mount(&site)
            .await;
    }
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let proposal = engine
        .detect_series_at(&format!("{}/book/index.html", site.uri()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proposal.title, "Ray Tracing Book");
    assert_eq!(proposal.chapters.len(), 4);
    let chapters: Vec<(String, String)> = proposal
        .chapters
        .iter()
        .map(|c| (c.title.clone(), c.url.clone()))
        .collect();
    let series_id = engine
        .create_series(&proposal.title, Some(&proposal.toc_url), &chapters)
        .unwrap();

    let info = engine.series_info(&series_id).unwrap().unwrap();
    assert_eq!((info.chapters, info.finished), (4, 0));
    let first = info.next_item_id.clone().unwrap();
    assert!(
        engine.items(&ItemQuery::default()).unwrap().is_empty(),
        "chapters stay out of the main list"
    );

    // Opening a chapter fetches its content on demand.
    let doc = engine.document(&first).await.unwrap().unwrap().document;
    let last = doc.blocks.last().unwrap().id.clone();
    engine
        .record_progress(&first, &last, 0, false, true)
        .unwrap();
    let second = engine
        .items(&ItemQuery {
            filter: ItemFilter::Series(series_id.clone()),
            ..Default::default()
        })
        .unwrap()[1]
        .item
        .id
        .clone();
    let mid = para_ids(&engine.document(&second).await.unwrap().unwrap().document)[5].clone();
    engine
        .record_progress(&second, &mid, 0, false, true)
        .unwrap();

    let info = engine.series_info(&series_id).unwrap().unwrap();
    assert_eq!(info.finished, 1);
    assert!(
        info.progress > 0.3 && info.progress < 0.5,
        "{}",
        info.progress
    );
    assert_eq!(info.next_item_id.as_deref(), Some(second.as_str()));
    assert_eq!(engine.series_list().unwrap().len(), 1);
}

#[tokio::test]
async fn local_search_covers_feed_and_saved_items() {
    let pages = mock_pages().await;
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();

    let feed_hits = engine.search("Unread elsewhere", 10).unwrap();
    assert_eq!(feed_hits[0].item.item.title, "Unread elsewhere");
    let saved_hits = engine.search("importance sampling", 10).unwrap();
    assert_eq!(saved_hits[0].item.item.title, "An Essay");
    assert!(!saved_hits[0].highlights.is_empty());
    assert!(
        engine.search("graphics blog", 10).unwrap().len() >= 3,
        "source names are searchable"
    );

    assert_eq!(engine.rebuild_search_index().unwrap(), 4);
    assert_eq!(engine.search("importance sampling", 10).unwrap().len(), 1);
}

fn epub_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(
        "mimetype",
        options.compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(b"application/epub+zip").unwrap();
    for (name, body) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

pub(crate) fn sample_epub() -> Vec<u8> {
    epub_bytes(&[
        (
            "META-INF/container.xml",
            br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        ),
        (
            "OEBPS/content.opf",
            br#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Sampling Book</dc:title><dc:creator>Ada</dc:creator></metadata>
<manifest>
  <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
  <item id="cover" href="text/cover.xhtml" media-type="application/xhtml+xml"/>
  <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
  <item id="c1b" href="text/ch1b.xhtml" media-type="application/xhtml+xml"/>
  <item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>
  <item id="img" href="images/cover.png" media-type="image/png" properties="cover-image"/>
  <item id="fig" href="images/fig.png" media-type="image/png"/>
</manifest>
<spine><itemref idref="cover"/><itemref idref="c1"/><itemref idref="c1b"/><itemref idref="c2"/></spine>
</package>"#,
        ),
        (
            "OEBPS/nav.xhtml",
            br#"<html><body><nav epub:type="toc"><ol><li><a href="text/ch1.xhtml">Chapter One</a></li><li><a href="text/ch2.xhtml">Chapter Two</a></li></ol></nav></body></html>"#,
        ),
        (
            "OEBPS/text/cover.xhtml",
            br#"<html><body><img src="../images/cover.png" alt="Cover"/></body></html>"#,
        ),
        (
            "OEBPS/text/ch1.xhtml",
            br#"<html><head><title/></head><body><h1>Chapter One</h1><p>Importance sampling draws samples where the integrand is large. See <a href="ch2.xhtml#n1">the note</a>.</p><img src="../images/fig.png" alt="Figure"/></body></html>"#,
        ),
        (
            "OEBPS/text/ch1b.xhtml",
            br#"<html><body><p>The estimator stays unbiased.</p></body></html>"#,
        ),
        (
            "OEBPS/text/ch2.xhtml",
            br#"<html><body><h1>Chapter Two</h1><p id="n1">Multiple importance sampling combines strategies.</p></body></html>"#,
        ),
        ("OEBPS/images/cover.png", PNG),
        ("OEBPS/images/fig.png", PNG),
    ])
}

#[tokio::test]
async fn epub_import_creates_a_publication_of_readable_chapters() {
    use eddy_model::{ContentFormat, ContentOrigin, ItemKind};
    let dir = tempfile::tempdir().unwrap();
    let engine = open(&dir.path().join("data"));
    let file = dir.path().join("sampling.epub");
    std::fs::write(&file, sample_epub()).unwrap();

    let imported = engine.import_file(&file).await.unwrap();
    assert!(!imported.existing);
    assert_eq!(imported.title, "Sampling Book");
    let series_id = imported.series_id.clone().unwrap();
    let info = engine.series_info(&series_id).unwrap().unwrap();
    assert_eq!(info.author.as_deref(), Some("Ada"));
    assert_eq!(info.chapters, 3);
    let cover = info.cover.clone().unwrap();
    assert!(
        cover.starts_with("epub://") && cover.ends_with("/OEBPS/images/cover.png"),
        "{cover}"
    );
    assert_eq!(
        engine.image(&cover).await.unwrap(),
        (PNG.to_vec(), "image/png".to_string())
    );

    let chapters = engine
        .items(&ItemQuery {
            filter: ItemFilter::Series(series_id.clone()),
            ..Default::default()
        })
        .unwrap();
    let titles: Vec<_> = chapters.iter().map(|r| r.item.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Sampling Book", "Chapter One", "Chapter Two"],
        "an untitled cover takes the book title; split files join their chapter"
    );
    assert!(chapters.iter().all(|r| r.item.kind == ItemKind::Chapter
        && r.format == ContentFormat::Article
        && r.item.author.as_deref() == Some("Ada")));
    assert_eq!(imported.item_id, chapters[0].item.id);
    assert!(
        engine.items(&ItemQuery::default()).unwrap().is_empty(),
        "chapters live in their book"
    );

    let one = chapters[1].item.id.clone();
    let doc = engine.document(&one).await.unwrap().unwrap();
    assert_eq!(doc.version.origin, ContentOrigin::Imported);
    let text = doc.document.plain_text();
    assert!(
        !text.starts_with("Chapter One"),
        "the title heading is dropped"
    );
    assert!(
        text.contains("integrand") && text.contains("stays unbiased"),
        "{text}"
    );
    let image = doc
        .document
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Image { src, .. } => Some(src.clone()),
            _ => None,
        })
        .unwrap();
    assert!(image.ends_with("/OEBPS/images/fig.png"), "{image}");
    assert_eq!(engine.image(&image).await.unwrap().1, "image/png");
    let link = doc
        .document
        .blocks
        .iter()
        .flat_map(|b| b.kind.runs().unwrap_or_default())
        .find_map(|r| r.link.clone())
        .unwrap();
    assert_eq!(
        engine.item_for_link(&link).unwrap().as_deref(),
        Some(chapters[2].item.id.as_str())
    );
    let continuation = link.replace("ch2.xhtml#n1", "ch1b.xhtml");
    assert_eq!(
        engine.item_for_link(&continuation).unwrap().as_deref(),
        Some(one.as_str()),
        "a continuation file belongs to its chapter"
    );
    assert_eq!(
        engine.search("combines strategies", 10).unwrap()[0]
            .item
            .item
            .id,
        chapters[2].item.id
    );

    // Documents regenerate from the stored book.
    engine
        .inner
        .store
        .set_doc_blob(
            &doc.version.hash,
            doc.version.doc_blob.as_deref().unwrap(),
            0,
            0,
        )
        .unwrap();
    let regenerated = engine.document(&one).await.unwrap().unwrap();
    assert_eq!(regenerated.document, doc.document);

    let again = engine.import_file(&file).await.unwrap();
    assert!(again.existing);
    assert_eq!(again.series_id, Some(series_id));
    assert_eq!(engine.series_list().unwrap().len(), 1);
}

struct FakePdfReader;

impl PdfReader for FakePdfReader {
    fn read(&self, path: &std::path::Path) -> Option<PdfText> {
        use crate::{PdfLine, PdfPage};
        // Full-width 10 pt body lines stacked 12 pt apart from `top`.
        let lines = |texts: &[&str], top: f32| -> Vec<PdfLine> {
            texts
                .iter()
                .enumerate()
                .map(|(i, t)| PdfLine {
                    text: t.to_string(),
                    x: 72.0,
                    y: top - 12.0 * i as f32,
                    width: 468.0,
                    height: 11.5,
                    font_size: 10.0,
                    bold: false,
                    runs: Vec::new(),
                })
                .collect()
        };
        let page = |lines: Vec<PdfLine>| PdfPage {
            width: 612.0,
            height: 792.0,
            lines,
            images: Vec::new(),
            frames: Vec::new(),
            rules: Vec::new(),
            boxes: Vec::new(),
        };
        let bytes = std::fs::read(path).ok()?;
        bytes.starts_with(b"%PDF-").then(|| {
            let mut first = lines(
                &[
                    "Importance sampling draws more samples where the inte-",
                    "grand is large and so reduces the variance of Monte",
                    "Carlo estimates without adding any bias at all.",
                ],
                700.0,
            );
            first.push(PdfLine {
                text: "A Short Heading".into(),
                x: 72.0,
                y: 660.0,
                width: 90.0,
                height: 16.0,
                font_size: 14.0,
                bold: true,
                runs: Vec::new(),
            });
            first.push(PdfLine {
                text: "Next paragraph.".into(),
                x: 72.0,
                y: 640.0,
                width: 80.0,
                height: 11.5,
                font_size: 10.0,
                bold: false,
                runs: Vec::new(),
            });
            let mut second = page(lines(&["第二页的内容比较长一些", "继续"], 700.0));
            second.images.push(crate::PdfImage {
                index: 4,
                x: 72.0,
                y: 300.0,
                width: 300.0,
                height: 200.0,
                pixel_width: 900,
                pixel_height: 600,
                fingerprint: 1,
            });
            PdfText {
                title: Some("Microsoft Word - draft.docx".into()),
                author: Some("Ada".into()),
                pages: vec![page(first), second],
            }
        })
    }

    fn image(&self, path: &std::path::Path, page: u32, index: u32) -> Option<(Vec<u8>, &'static str)> {
        std::fs::read(path).ok()?.starts_with(b"%PDF-").then_some(())?;
        (page == 1 && index == 4).then(|| (b"\x89PNG figure".to_vec(), "image/png"))
    }
}

#[tokio::test]
async fn pdf_import_reads_text_through_the_platform_and_tracks_pages() {
    use eddy_model::{ContentFormat, ItemKind};
    let dir = tempfile::tempdir().unwrap();
    let engine = open(&dir.path().join("data"));
    engine.set_pdf_reader(Some(Arc::new(FakePdfReader)));
    let pdf = b"%PDF-1.7\n1 0 obj<<>>endobj\n%%EOF\n".to_vec();
    let file = dir.path().join("Paper Draft.pdf");
    std::fs::write(&file, &pdf).unwrap();

    let imported = engine.import_file(&file).await.unwrap();
    assert_eq!(
        imported.title, "Paper Draft",
        "authoring-tool titles are ignored"
    );
    assert_eq!(imported.series_id, None);
    let row = engine.item(&imported.item_id).unwrap().unwrap();
    assert_eq!(
        (row.item.kind, row.format),
        (ItemKind::Saved, ContentFormat::Pdf)
    );
    assert_eq!(row.item.author.as_deref(), Some("Ada"));
    assert!(row.item.url.is_none());
    assert!(row.item.excerpt.unwrap().starts_with("Importance sampling"));

    let doc = engine
        .document(&imported.item_id)
        .await
        .unwrap()
        .unwrap()
        .document;
    let paragraphs: Vec<String> = doc.blocks.iter().map(|b| b.plain_text()).collect();
    assert_eq!(
        paragraphs,
        [
            "Importance sampling draws more samples where the integrand is large and so reduces the variance of Monte Carlo estimates without adding any bias at all.",
            "A Short Heading",
            "Next paragraph.",
            "第二页的内容比较长一些继续",
            "",
        ]
    );
    let figure = doc
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            eddy_model::BlockKind::Image { src, .. } => Some(src.clone()),
            _ => None,
        })
        .expect("the figure on page 2");
    assert!(figure.starts_with("pdf://") && figure.ends_with("/1/4"), "{figure}");
    // Importing started rendering the figure into the image cache.
    let cached = crate::images::image_blob_key(&figure);
    for _ in 0..200 {
        if engine.inner.store.has_blob(&cached).unwrap() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(engine.inner.store.has_blob(&cached).unwrap(), "figure warmed on import");
    let (bytes, mime) = engine.image(&figure).await.unwrap();
    assert_eq!((bytes.as_slice(), mime.as_str()), (&b"\x89PNG figure"[..], "image/png"));
    assert!(engine.image(&figure.replace("/1/4", "/1/5")).await.is_err());
    assert_eq!(
        engine.search("integrand", 5).unwrap()[0].item.item.id,
        imported.item_id
    );

    let path = engine
        .content_file(&imported.item_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), pdf);

    engine
        .record_page_progress(&imported.item_id, 1, 4, true)
        .unwrap();
    let first_block = doc.blocks[0].id.clone();
    assert_eq!(
        engine
            .record_progress(&imported.item_id, &first_block, 0, false, true)
            .unwrap(),
        None,
        "the text view does not replace the page"
    );
    let resume = engine.resume_point(&imported.item_id).unwrap().unwrap();
    assert_eq!((resume.page, resume.block_id), (Some(1), None));
    assert!((resume.locator.progression - 0.25).abs() < 1e-9);
    assert!(
        !engine
            .item(&imported.item_id)
            .unwrap()
            .unwrap()
            .item
            .finished
    );
    engine
        .record_page_progress(&imported.item_id, 3, 4, true)
        .unwrap();
    assert!(
        engine
            .item(&imported.item_id)
            .unwrap()
            .unwrap()
            .item
            .finished
    );

    assert!(engine.import_file(&file).await.unwrap().existing);
    let saved = engine
        .items(&ItemQuery {
            filter: ItemFilter::Saved,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(saved.len(), 1);
}

#[tokio::test]
async fn imports_without_text_and_unsupported_files() {
    use eddy_model::ContentFormat;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(&dir.path().join("data"));

    let scan = dir.path().join("scan.pdf");
    std::fs::write(&scan, b"%PDF-1.4\n%%EOF\n").unwrap();
    let imported = engine.import_file(&scan).await.unwrap();
    assert_eq!(imported.title, "scan");
    let doc = engine.document(&imported.item_id).await.unwrap().unwrap();
    assert!(
        doc.document.blocks.is_empty(),
        "without a PDF reader there is no text, but the file opens"
    );
    assert_eq!(
        engine.item(&imported.item_id).unwrap().unwrap().format,
        ContentFormat::Pdf
    );

    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "plain text").unwrap();
    assert!(matches!(
        engine.import_file(&notes).await,
        Err(EngineError::Invalid(_))
    ));

    let locked = dir.path().join("locked.epub");
    std::fs::write(
        &locked,
        epub_bytes(&[(
            "META-INF/encryption.xml",
            br#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></EncryptedData></encryption>"#,
        )]),
    )
    .unwrap();
    let err = engine.import_file(&locked).await.unwrap_err().to_string();
    assert!(err.contains("DRM"), "{err}");
    assert!(engine.series_list().unwrap().is_empty());
}

fn entry<'a>(entries: &'a [CollectionEntry], title: &str) -> &'a CollectionEntry {
    entries
        .iter()
        .find(|e| e.title == title)
        .unwrap_or_else(|| panic!("no entry {title}"))
}

#[tokio::test]
async fn the_collection_holds_books_pdfs_pages_and_series_and_deletes_them() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(&dir.path().join("data"));
    engine.set_pdf_reader(Some(Arc::new(FakePdfReader)));
    let epub = dir.path().join("sampling.epub");
    std::fs::write(&epub, sample_epub()).unwrap();
    let book = engine.import_file(&epub).await.unwrap();
    let pdf = dir.path().join("Paper Draft.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\n1 0 obj<<>>endobj\n%%EOF\n").unwrap();
    let paper = engine.import_file(&pdf).await.unwrap();
    let essay = engine
        .save_url(&format!("{}/essay", pages.uri()))
        .await
        .unwrap();
    let adopted = engine
        .save_url(&format!("{}/1", pages.uri()))
        .await
        .unwrap();
    let contents = engine
        .save_url(&format!("{}/3", pages.uri()))
        .await
        .unwrap();
    let series_id = engine
        .create_series(
            "Notes",
            Some(&format!("{}/3", pages.uri())),
            &[
                ("One".into(), format!("{}/1", pages.uri())),
                ("Two".into(), format!("{}/2", pages.uri())),
            ],
        )
        .unwrap();

    let entries = engine.collection().unwrap();
    assert_eq!(
        entries.len(),
        4,
        "neither a page that joined a series nor its table of contents is listed on its own"
    );
    let sampling = entry(&entries, "Sampling Book");
    assert_eq!(
        (
            sampling.kind,
            sampling.chapters,
            sampling.series_id.as_deref()
        ),
        (CollectionKind::Book, 3, book.series_id.as_deref())
    );
    assert_eq!(
        sampling.resume_item_id.as_deref(),
        Some(book.item_id.as_str()),
        "an unread book opens at its first chapter"
    );
    assert!(sampling.text_len > 0 && sampling.cover.is_some());
    let draft = entry(&entries, "Paper Draft");
    assert_eq!((draft.kind, draft.chapters), (CollectionKind::Pdf, 1));
    assert_eq!(draft.id, paper.item_id);
    assert_eq!(
        draft.resume_item_id.as_deref(),
        Some(paper.item_id.as_str())
    );
    let page = entry(&entries, "An Essay");
    assert_eq!(
        (page.kind, page.id.as_str()),
        (CollectionKind::WebPage, essay.item.id.as_str())
    );
    assert!(page.url.as_deref().unwrap().ends_with("/essay"));
    let notes = entry(&entries, "Notes");
    assert_eq!((notes.kind, notes.chapters), (CollectionKind::WebSeries, 2));

    // Reading part of the book resumes there.
    let chapters = engine
        .items(&ItemQuery {
            filter: ItemFilter::Series(book.series_id.clone().unwrap()),
            ..Default::default()
        })
        .unwrap();
    let second = &chapters[1].item.id;
    let doc = engine.document(second).await.unwrap().unwrap().document;
    engine
        .record_progress(second, &doc.blocks[0].id, 0, false, true)
        .unwrap();
    let entries = engine.collection().unwrap();
    let sampling = entry(&entries, "Sampling Book");
    assert_eq!(sampling.resume_item_id.as_deref(), Some(second.as_str()));
    assert!(sampling.last_read_at.is_some());

    // Deleting a book removes its chapters, their search entries and the file.
    let book_blob = engine
        .document(&book.item_id)
        .await
        .unwrap()
        .unwrap()
        .version
        .raw_blob;
    assert!(!engine.search("combines strategies", 10).unwrap().is_empty());
    engine.delete_entry(&sampling.id).await.unwrap();
    assert!(engine.series_info(&sampling.id).unwrap().is_none());
    assert!(
        engine
            .items(&ItemQuery {
                filter: ItemFilter::Series(sampling.id.clone()),
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(engine.search("combines strategies", 10).unwrap().is_empty());
    assert!(!engine.inner.store.has_blob(&book_blob).unwrap());
    let reimported = engine.import_file(&epub).await.unwrap();
    assert!(!reimported.existing, "a deleted book imports afresh");
    assert_ne!(reimported.series_id, book.series_id);

    // A web series takes the pages that joined it and its table of contents along.
    engine.delete_series(&series_id).unwrap();
    let saved = engine
        .items(&ItemQuery {
            filter: ItemFilter::Saved,
            ..Default::default()
        })
        .unwrap();
    assert!(
        saved
            .iter()
            .all(|r| r.item.id != adopted.item.id && r.item.id != contents.item.id)
    );

    engine.delete_entry(&paper.item_id).await.unwrap();
    assert!(engine.content_file(&paper.item_id).await.unwrap().is_none());
    engine.delete_entry(&essay.item.id).await.unwrap();
    let left: Vec<_> = engine
        .collection()
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect();
    assert_eq!(left, ["Sampling Book"]);
    assert!(matches!(
        engine.delete_entry("missing").await,
        Err(EngineError::NotFound)
    ));
}

#[tokio::test]
async fn subscription_articles_cannot_be_deleted() {
    let server = mock_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine
        .configure_freshrss(&server.uri(), "me", "secret")
        .await
        .unwrap();
    engine.refresh().await.unwrap();
    let article = engine.items(&ItemQuery::default()).unwrap()[0]
        .item
        .id
        .clone();
    assert!(matches!(
        engine.delete_entry(&article).await,
        Err(EngineError::Invalid(_))
    ));
    assert!(engine.collection().unwrap().is_empty());
}
