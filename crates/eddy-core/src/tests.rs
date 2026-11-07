use std::sync::Arc;

use eddy_model::BlockKind;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path, path_regex, query_param};
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

fn article_html(title: &str, paragraphs: usize) -> String {
    let body: String = (0..paragraphs)
        .map(|i| format!("<p>Paragraph {i} of {title} explains importance sampling and why the estimator stays unbiased.</p>"))
        .collect();
    format!("<html><head><title>{title} | Site</title></head><body><nav><a href='/'>Home</a></nav><article><h1>{title}</h1>{body}</article><footer>Footer</footer></body></html>")
}

async fn mock_pages() -> MockServer {
    let server = MockServer::start().await;
    for n in 1..=3 {
        Mock::given(method("GET"))
            .and(path(format!("/{n}")))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(article_html(&format!("Full {n}"), 30)))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/essay"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(article_html("An Essay", 20)))
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
    engine.configure_freshrss(&server.uri(), "me", "secret").await.unwrap();
    engine.refresh().await.unwrap();
    let row = engine.items(&ItemQuery::default()).unwrap().into_iter().find(|r| r.item.title == "New post").unwrap();
    let before = engine.document(&row.item.id).await.unwrap().unwrap();

    let full = engine.fetch_full_text(&row.item.id).await.unwrap();
    assert_eq!(full.version.origin, eddy_model::ContentOrigin::Extracted);
    assert!(full.document.text_len() > before.document.text_len() * 3);
    assert!(!full.document.plain_text().contains("Footer"));

    let current = engine.document(&row.item.id).await.unwrap().unwrap();
    assert_eq!(current.version.hash, full.version.hash);
    let source = engine.sources().unwrap().into_iter().find(|s| s.source.title == "Graphics Blog").unwrap();
    assert!(source.source.truncated);
    // Switching versions is an op, so it syncs like any other change.
    assert!(engine.inner.store.unuploaded_ops(100).unwrap().iter().any(|o| o.op.field == "version"));
}

#[tokio::test]
async fn saving_urls_and_importing_captures() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());

    let saved = engine.save_url(&format!("{}/essay", pages.uri())).await.unwrap();
    assert_eq!(saved.item.title, "An Essay");
    assert_eq!(saved.item.kind, eddy_model::ItemKind::Saved);
    let again = engine.save_url(&format!("{}/essay", pages.uri())).await.unwrap();
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
    let saved_items = engine.items(&ItemQuery { filter: ItemFilter::Saved, ..Default::default() }).unwrap();
    let captured = saved_items.iter().find(|r| r.item.title == "Members only").unwrap();
    let doc = engine.document(&captured.item.id).await.unwrap().unwrap();
    assert_eq!(doc.version.origin, eddy_model::ContentOrigin::Captured);

    // Evicting the derived document regenerates it by re-extracting the page.
    let blob = doc.version.doc_blob.clone().unwrap();
    std::fs::remove_file(dir.path().join("blobs").join(&blob[blob.len() - 2..]).join(&blob)).unwrap();
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
    assert!(engine.import_rule_pack("../escape", "name = \"x\"").is_err());
    let summary = engine.import_ftr_rule(".example.org", "body: //article\n").unwrap();
    assert_eq!(summary.ftr_sites, 1);
    assert_eq!(engine.inner.fetcher.read().rule_for("https://www.example.com/a").unwrap().strategies, ["archive"]);
    assert!(engine.inner.fetcher.read().rule_for("https://news.example.org/a").is_some());
}

async fn mock_llm() -> MockServer {
    let server = MockServer::start().await;
    let reply = |content: serde_json::Value| {
        ResponseTemplate::new(200).set_body_json(json!({"choices": [{"message": {"content": content.to_string()}}]}))
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
        .respond_with(reply(json!({"tags": ["Rendering", "Rust"], "suggestions": ["Sampling"]})))
        .mount(&server)
        .await;
    // Translation: echo every segment ID back with a prefix.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("Translate each input segment"))
        .respond_with(|req: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let user: serde_json::Value = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let translations: Vec<_> = user["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| json!({"id": s["id"], "text": format!("译：{}", s["text"].as_str().unwrap())}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({"choices": [{"message": {"content": json!({"translations": translations}).to_string()}}]}))
        })
        .mount(&server)
        .await;
    server
}

fn configure_ai(engine: &Engine, llm: &MockServer, rules: Vec<AutoRule>) {
    engine
        .set_ai_settings(AiSettings {
            provider: Some(ProviderSettings { base_url: llm.uri(), model: "test-model".into() }),
            rules,
            ..Default::default()
        })
        .unwrap();
    engine.set_ai_key(Some("sk-test-secret-key")).unwrap();
}

#[tokio::test]
async fn translation_summary_and_tags_with_caching() {
    let pages = mock_pages().await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(&engine, &llm, vec![]);
    assert!(engine.ai_settings().unwrap().provider.unwrap().base_url.ends_with("/v1"));
    let row = engine.save_url(&format!("{}/essay", pages.uri())).await.unwrap();

    let batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = batches.clone();
    let callback: BatchCallback = Arc::new(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let t = engine.translate(&row.item.id, false, Some(callback)).await.unwrap();
    assert!(!t.cached);
    assert_eq!(t.segments["title"], "译：An Essay");
    let doc = engine.document(&row.item.id).await.unwrap().unwrap().document;
    let first_para = doc.blocks.iter().find(|b| matches!(b.kind, BlockKind::Paragraph { .. })).unwrap();
    assert!(t.segments[&first_para.id].starts_with("译：Paragraph 0"));
    assert!(batches.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    let requests_after_first = llm.received_requests().await.unwrap().len();

    let again = engine.translate(&row.item.id, false, None).await.unwrap();
    assert!(again.cached);
    assert_eq!(llm.received_requests().await.unwrap().len(), requests_after_first);
    assert_eq!(engine.cached_translation(&row.item.id).unwrap().unwrap().segments, t.segments);

    let s = engine.summarize(&row.item.id, false).await.unwrap();
    assert_eq!(s.summary.tldr, "一句话");
    assert!(engine.summarize(&row.item.id, false).await.unwrap().cached);

    let rendering = engine.create_tag("Rendering").unwrap();
    assert_eq!(engine.create_tag(" rendering ").unwrap(), rendering);
    let tags = engine.suggest_tags(&row.item.id).await.unwrap();
    assert_eq!(tags.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Rendering"]);
    assert_eq!(tags.suggestions, ["Rust", "Sampling"]);
    engine.accept_tag_suggestion(&row.item.id, "Sampling").unwrap();
    engine.set_item_tag(&row.item.id, &rendering, false).unwrap();
    let after = engine.suggest_tags(&row.item.id).await.unwrap();
    assert_eq!(after.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Sampling"]);
    assert_eq!(engine.items(&ItemQuery { filter: ItemFilter::Tag(eddy_model::ids::tag_id("sampling")), ..Default::default() }).unwrap().len(), 1);

    // The key lives in the keychain, never in the database.
    let db = std::fs::read(dir.path().join("eddy.sqlite3")).unwrap();
    let wal = std::fs::read(dir.path().join("eddy.sqlite3-wal")).unwrap_or_default();
    let needle = b"sk-test-secret-key";
    assert!(!db.windows(needle.len()).any(|w| w == needle) && !wal.windows(needle.len()).any(|w| w == needle));
}

#[tokio::test]
async fn automatic_rules_run_on_new_items() {
    let pages = mock_pages().await;
    let llm = mock_llm().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    configure_ai(&engine, &llm, vec![AutoRule { scope: RuleScope::All, action: AutoAction::Summarize, min_chars: 100, only_foreign: true }]);
    let row = engine.save_url(&format!("{}/essay", pages.uri())).await.unwrap();
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

fn para_ids(doc: &eddy_model::Document) -> Vec<String> {
    doc.blocks.iter().filter(|b| matches!(b.kind, BlockKind::Paragraph { .. })).map(|b| b.id.clone()).collect()
}

#[tokio::test]
async fn progress_is_throttled_finishes_and_feeds_continue_reading() {
    let pages = mock_pages().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let row = engine.save_url(&format!("{}/essay", pages.uri())).await.unwrap();
    let doc = engine.document(&row.item.id).await.unwrap().unwrap().document;
    let paras = para_ids(&doc);
    let ops = |e: &Engine| e.inner.store.unuploaded_ops(10_000).unwrap().iter().filter(|o| o.op.field == "locator").count();

    let mid = engine.record_progress(&row.item.id, &paras[paras.len() / 2], 3, false).unwrap().unwrap();
    assert!(mid.progression > 0.3 && mid.progression < 0.7, "{}", mid.progression);
    engine.record_progress(&row.item.id, &paras[paras.len() / 2 + 1], 0, false).unwrap();
    assert_eq!(ops(&engine), 1, "scrolling within the interval is throttled");
    engine.record_progress(&row.item.id, &paras[paras.len() / 2 + 1], 0, true).unwrap();
    assert_eq!(ops(&engine), 2);
    let shelf = engine.items(&ItemQuery { filter: ItemFilter::ContinueReading, ..Default::default() }).unwrap();
    assert_eq!(shelf.len(), 1);

    let resume = engine.resume_point(&row.item.id).unwrap().unwrap();
    assert_eq!(resume.block_id.as_deref(), Some(paras[paras.len() / 2 + 1].as_str()));
    assert!(!resume.from_other_device);

    let last = doc.blocks.last().unwrap().id.clone();
    engine.record_progress(&row.item.id, &last, 0, true).unwrap();
    assert!(engine.item(&row.item.id).unwrap().unwrap().item.finished);
    assert!(engine.items(&ItemQuery { filter: ItemFilter::ContinueReading, ..Default::default() }).unwrap().is_empty());
}

#[tokio::test]
async fn resume_point_follows_the_switch_to_full_text() {
    let pages = MockServer::start().await;
    // Feed excerpt and full page share the second paragraph.
    let shared = "The shared paragraph describes multiple importance sampling in detail for the reader.";
    let body: String = (0..25).map(|i| format!("<p>Full paragraph {i} adds detail about light transport and variance.</p>")).collect();
    Mock::given(method("GET"))
        .and(path("/2"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(format!(
            "<html><head><title>New post</title></head><body><article><p>Intro only in the full text.</p><p>{shared}</p>{body}</article></body></html>"
        )))
        .mount(&pages)
        .await;
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/greader.php/accounts/ClientLogin")).respond_with(ResponseTemplate::new(200).set_body_string("Auth=t")).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/greader.php/reader/api/0/subscription/list")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"subscriptions": []}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/greader.php/reader/api/0/stream/items/ids")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"itemRefs": [{"id": "2"}]}))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path_regex("^/api/greader.php/reader/api/0/stream/contents/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{
            "id": "tag:google.com,2005:reader/item/0000000000000002", "title": "New post", "published": 1_700_000_000,
            "categories": [], "alternate": [{"href": format!("{}/2", pages.uri())}],
            "summary": {"content": format!("<p>Teaser line.</p><p>{shared}</p>")}, "origin": {"streamId": "feed/1"}
        }]})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/api/greader.php/reader/api/0/stream/items/contents")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []}))).mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    engine.configure_freshrss(&server.uri(), "me", "pw").await.unwrap();
    engine.refresh().await.unwrap();
    let item = engine.items(&ItemQuery::default()).unwrap().remove(0);
    let feed_doc = engine.document(&item.item.id).await.unwrap().unwrap().document;
    let shared_id = para_ids(&feed_doc)[1].clone();
    engine.record_progress(&item.item.id, &shared_id, 13, true).unwrap();

    let full = engine.fetch_full_text(&item.item.id).await.unwrap();
    let resume = engine.resume_point(&item.item.id).unwrap().unwrap();
    assert_eq!(resume.locator.content_hash, full.version.hash);
    assert_eq!(resume.block_id.as_deref(), Some(shared_id.as_str()), "same paragraph keeps its ID across versions");
    assert_eq!(resume.offset, 13);
}

#[tokio::test]
async fn series_from_a_table_of_contents() {
    let site = MockServer::start().await;
    let toc: String = (1..=4).map(|i| format!("<li><a href=\"/book/ch{i}.html\">Chapter {i}</a></li>")).collect();
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
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(article_html(&format!("Chapter {i}"), 12)))
            .mount(&site)
            .await;
    }
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let proposal = engine.detect_series_at(&format!("{}/book/index.html", site.uri())).await.unwrap().unwrap();
    assert_eq!(proposal.title, "Ray Tracing Book");
    assert_eq!(proposal.chapters.len(), 4);
    let chapters: Vec<(String, String)> = proposal.chapters.iter().map(|c| (c.title.clone(), c.url.clone())).collect();
    let series_id = engine.create_series(&proposal.title, Some(&proposal.toc_url), &chapters).unwrap();

    let info = engine.series_info(&series_id).unwrap().unwrap();
    assert_eq!((info.chapters, info.finished), (4, 0));
    let first = info.next_item_id.clone().unwrap();
    assert!(engine.items(&ItemQuery::default()).unwrap().is_empty(), "chapters stay out of the main list");

    // Opening a chapter fetches its content on demand.
    let doc = engine.document(&first).await.unwrap().unwrap().document;
    let last = doc.blocks.last().unwrap().id.clone();
    engine.record_progress(&first, &last, 0, true).unwrap();
    let second = engine.items(&ItemQuery { filter: ItemFilter::Series(series_id.clone()), ..Default::default() }).unwrap()[1].item.id.clone();
    let mid = para_ids(&engine.document(&second).await.unwrap().unwrap().document)[5].clone();
    engine.record_progress(&second, &mid, 0, true).unwrap();

    let info = engine.series_info(&series_id).unwrap().unwrap();
    assert_eq!(info.finished, 1);
    assert!(info.progress > 0.3 && info.progress < 0.5, "{}", info.progress);
    assert_eq!(info.next_item_id.as_deref(), Some(second.as_str()));
    assert_eq!(engine.series_list().unwrap().len(), 1);
}
