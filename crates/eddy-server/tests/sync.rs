//! End-to-end sync: a standalone client migrates to a server, a second client
//! (separate data directory) joins, and changes flow between them and back to
//! FreshRSS.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eddy_core::{Engine, EngineOptions, ItemFilter, ItemQuery, MemoryPlatform, Mode};
use eddy_server::{AppState, ServerConfig, create_pairing_code, router};
use serde_json::json;
use wiremock::matchers::{method, path, path_regex, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const READ: &str = "user/-/state/com.google/read";
const STARRED: &str = "user/-/state/com.google/starred";

#[derive(Default)]
struct FreshState {
    listed: Vec<u64>,
    unread: HashSet<u64>,
    starred: HashSet<u64>,
}

/// FreshRSS mock whose read/starred state changes with edit-tag calls.
async fn stateful_freshrss() -> (MockServer, Arc<Mutex<FreshState>>) {
    let server = MockServer::start().await;
    let state = Arc::new(Mutex::new(FreshState {
        listed: vec![1, 2, 3],
        unread: [2, 3].into(),
        starred: [1].into(),
    }));
    let titles = [
        (1u64, "Old post"),
        (2, "New post"),
        (3, "Unread elsewhere"),
        (4, "Read on the web"),
    ];

    let item = move |st: &FreshState, n: u64| {
        let title = titles.iter().find(|(i, _)| *i == n).unwrap().1;
        let mut cats = vec!["user/-/state/com.google/reading-list".to_string()];
        if !st.unread.contains(&n) {
            cats.push(READ.into());
        }
        if st.starred.contains(&n) {
            cats.push(STARRED.into());
        }
        json!({
            "id": format!("tag:google.com,2005:reader/item/{n:016x}"), "title": title, "published": 1_700_000_000 + n,
            "categories": cats, "alternate": [{"href": format!("https://blog.example/{n}")}],
            "summary": {"content": format!("<p>Body of {title} about path tracing and radiance.</p>")},
            "origin": {"streamId": "feed/1", "title": "Graphics Blog"}
        })
    };

    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Auth=t\n"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("T"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/subscription/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"subscriptions": [{"id": "feed/1", "title": "Graphics Blog", "categories": [{"label": "Tech"}]}]})))
        .mount(&server)
        .await;
    let st = state.clone();
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .respond_with(move |req: &Request| {
            let q: std::collections::HashMap<String, String> = req.url.query_pairs().into_owned().collect();
            let s = st.lock().unwrap();
            let ids: Vec<u64> = if q.get("s").map(String::as_str) == Some(STARRED) { s.starred.iter().copied().collect() } else { s.unread.iter().copied().collect() };
            ResponseTemplate::new(200).set_body_json(json!({"itemRefs": ids.iter().map(|i| json!({"id": i.to_string()})).collect::<Vec<_>>()}))
        })
        .mount(&server)
        .await;
    let st = state.clone();
    let it = item;
    Mock::given(method("GET"))
        .and(path_regex(
            "^/api/greader.php/reader/api/0/stream/contents/.*",
        ))
        .respond_with(move |_: &Request| {
            let s = st.lock().unwrap();
            let items: Vec<_> = s.listed.iter().map(|n| it(&s, *n)).collect();
            ResponseTemplate::new(200).set_body_json(json!({ "items": items }))
        })
        .mount(&server)
        .await;
    let st = state.clone();
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/stream/items/contents"))
        .respond_with(move |req: &Request| {
            let s = st.lock().unwrap();
            let items: Vec<_> = url::form_urlencoded::parse(&req.body)
                .filter(|(k, _)| k == "i")
                .filter_map(|(_, v)| u64::from_str_radix(v.rsplit('/').next()?, 16).ok())
                .filter(|n| s.listed.contains(n))
                .map(|n| item(&s, n))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({ "items": items }))
        })
        .mount(&server)
        .await;
    let st = state.clone();
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/edit-tag"))
        .respond_with(move |req: &Request| {
            let form: Vec<(String, String)> = url::form_urlencoded::parse(&req.body)
                .into_owned()
                .collect();
            let ids: Vec<u64> = form
                .iter()
                .filter(|(k, _)| k == "i")
                .filter_map(|(_, v)| u64::from_str_radix(v.rsplit('/').next()?, 16).ok())
                .collect();
            let mut s = st.lock().unwrap();
            for (k, tag) in form.iter().filter(|(k, _)| k == "a" || k == "r") {
                for id in &ids {
                    let add = k == "a";
                    match (tag.as_str(), add) {
                        (READ, true) => {
                            s.unread.remove(id);
                        }
                        (READ, false) => {
                            s.unread.insert(*id);
                        }
                        (STARRED, true) => {
                            s.starred.insert(*id);
                        }
                        (STARRED, false) => {
                            s.starred.remove(id);
                        }
                        _ => {}
                    }
                }
            }
            ResponseTemplate::new(200).set_body_string("OK")
        })
        .mount(&server)
        .await;
    (server, state)
}

async fn pages() -> MockServer {
    let server = MockServer::start().await;
    let body: String = (0..20).map(|i| format!("<p>Paragraph {i} on importance sampling and multiple importance sampling weights.</p>")).collect();
    Mock::given(method("GET"))
        .and(path("/essay"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(format!(
            "<html><head><title>Sampling Essay</title></head><body><article><h1>Sampling Essay</h1>{body}</article></body></html>"
        )))
        .mount(&server)
        .await;
    server
}

/// A one-chapter EPUB with an image.
fn book() -> Vec<u8> {
    use std::io::Write;
    let files: [(&str, &[u8]); 5] = [
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#),
        ("content.opf", br#"<package><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Path Tracing Notes</dc:title></metadata>
<manifest><item id="c" href="c.xhtml" media-type="application/xhtml+xml"/><item id="i" href="i.png" media-type="image/png"/></manifest>
<spine><itemref idref="c"/></spine></package>"#),
        ("c.xhtml", br#"<html><body><h1>Radiance</h1><p>Radiance is constant along rays in a vacuum.</p><img src="i.png"/></body></html>"#),
        ("i.png", b"\x89PNG-figure"),
        ("mimetype", b"application/epub+zip"),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn client(dir: &std::path::Path) -> Engine {
    Engine::open(
        dir,
        Arc::new(MemoryPlatform::default()),
        EngineOptions::default(),
    )
    .unwrap()
}

fn find(engine: &Engine, title: &str) -> eddy_core::ItemRow {
    let mut all = engine
        .items(&ItemQuery {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    all.retain(|r| r.item.title == title);
    all.pop().unwrap_or_else(|| panic!("{title} missing"))
}

async fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn standalone_client_migrates_and_a_second_device_joins() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let site = pages().await;

    // Client A starts standalone with real local data.
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    a.refresh().await.unwrap();
    let new_post = find(&a, "New post");
    a.set_read(&new_post.item.id, true).await.unwrap();
    a.refresh().await.unwrap();
    assert!(
        !fresh_state.lock().unwrap().unread.contains(&2),
        "standalone pushes to FreshRSS directly"
    );
    let essay = a.save_url(&format!("{}/essay", site.uri())).await.unwrap();
    let essay_doc = a.document(&essay.item.id).await.unwrap().unwrap().document;
    let mid = essay_doc.blocks[essay_doc.blocks.len() / 2].id.clone();
    a.record_progress(&essay.item.id, &mid, 4, false, true).unwrap();
    let tag = a.create_tag("Sampling").unwrap();
    a.set_item_tag(&essay.item.id, &tag, true).unwrap();
    let book_file = dir_a.path().join("notes.epub");
    std::fs::write(&book_file, book()).unwrap();
    let imported = a.import_file(&book_file).await.unwrap();
    a.import_rule_pack("private", "name = \"Private\"\nversion = 1\n[[site]]\ndomains = [\"example.com\"]\nstrategies = [\"archive\"]\n").unwrap();

    // The server starts empty.
    let server_dir = tempfile::tempdir().unwrap();
    let state = AppState::open(&ServerConfig {
        data_dir: server_dir.path().to_path_buf(),
        master_key: [9u8; 32],
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, router(state.clone())).into_future());

    // A migrates: its FreshRSS account and all local changes move over.
    let code = create_pairing_code(&state.store, Duration::from_secs(60)).unwrap();
    let status = a.connect_server(&url, &code, "Mac A").await.unwrap();
    assert_eq!(status.account.unwrap().username, "me");
    assert_eq!(a.status().unwrap().mode, Mode::Server);
    assert!(
        a.connect_server(&url, &code, "again").await.is_err(),
        "pairing codes are single use"
    );
    assert!(state.engine.server_account().is_some());
    let secrets = std::fs::read_to_string(server_dir.path().join("secrets.json")).unwrap();
    assert!(
        !secrets.contains("\"pw\""),
        "the FreshRSS password is encrypted at rest"
    );

    // B joins with its own data directory and receives everything.
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    let code_b = create_pairing_code(&state.store, Duration::from_secs(60)).unwrap();
    b.connect_server(&url, &code_b, "Mac B").await.unwrap();
    assert_eq!(b.items(&ItemQuery::default()).unwrap().len(), 4);
    assert!(find(&b, "New post").item.read, "read state from A");
    assert!(
        find(&b, "Old post").item.starred,
        "starred state from FreshRSS"
    );
    let b_essay = find(&b, "Sampling Essay");
    assert_eq!(b_essay.item.id, essay.item.id);
    let b_doc = b
        .document(&b_essay.item.id)
        .await
        .unwrap()
        .expect("blob fetched from the server")
        .document;
    assert_eq!(b_doc, essay_doc);
    let resume = b.resume_point(&b_essay.item.id).unwrap().unwrap();
    assert!(resume.from_other_device);
    assert_eq!(resume.block_id.as_deref(), Some(mid.as_str()));
    assert_eq!(resume.offset, 4);
    assert_eq!(
        b.item_tags(&b_essay.item.id).unwrap().tags[0].name,
        "Sampling"
    );
    assert_eq!(
        b.rule_packs().packs,
        vec![("Private".to_string(), 1)],
        "private rule packs are distributed"
    );
    // An imported book arrives with its chapters; the book file itself
    // (for images) comes from the server on demand.
    let books = b.series_list().unwrap();
    let notes = books
        .iter()
        .find(|s| s.title == "Path Tracing Notes")
        .unwrap();
    assert_eq!(
        notes.next_item_id.as_deref(),
        Some(imported.item_id.as_str())
    );
    let chapter = b
        .document(&imported.item_id)
        .await
        .unwrap()
        .expect("chapter document from the server")
        .document;
    assert!(chapter.plain_text().contains("constant along rays"));
    let figure = chapter
        .blocks
        .iter()
        .find_map(|b| match &b.kind {
            eddy_model::BlockKind::Image { src, .. } => Some(src.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(b.image(&figure).await.unwrap().0, b"\x89PNG-figure");

    let feed_doc = b
        .document(&find(&b, "Unread elsewhere").item.id)
        .await
        .unwrap()
        .unwrap()
        .document;
    assert!(feed_doc.plain_text().contains("Unread elsewhere"));

    // Server-side search over the full history.
    let hits = b.search_all("importance sampling", 10).await.unwrap();
    assert_eq!(hits[0].item.item.id, essay.item.id);

    // A listens live; B stars an item; the server pushes it to FreshRSS and A
    // picks it up without a manual refresh.
    a.start_live_sync();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let unread = find(&b, "Unread elsewhere");
    b.set_starred(&unread.item.id, true).await.unwrap();
    b.refresh().await.unwrap();
    assert!(
        eventually(|| fresh_state.lock().unwrap().starred.contains(&3)).await,
        "server pushed the star to FreshRSS"
    );
    let a_engine = a.clone();
    let id = unread.item.id.clone();
    assert!(
        eventually(move || a_engine.item(&id).unwrap().unwrap().item.starred).await,
        "live sync delivered the change to A"
    );

    // Concurrent progress on two devices: the later write wins everywhere.
    b.record_progress(&essay.item.id, &essay_doc.blocks[2].id, 0, false, true)
        .unwrap();
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    assert_eq!(
        a.resume_point(&essay.item.id)
            .unwrap()
            .unwrap()
            .block_id
            .as_deref(),
        Some(essay_doc.blocks[2].id.as_str())
    );
    let shelf = a
        .items(&ItemQuery {
            filter: ItemFilter::ContinueReading,
            ..Default::default()
        })
        .unwrap();
    assert!(
        shelf[0].item.progress < 0.2,
        "A's list shows B's later position, not its own earlier one"
    );
    a.stop_live_sync();

    // An article already read on the FreshRSS web arrives read and is not
    // announced (or auto-processed) as new.
    fresh_state.lock().unwrap().listed.push(4);
    assert_eq!(b.refresh().await.unwrap().new_items, 0);
    assert!(find(&b, "Read on the web").item.read);

    // Deleting the book on A removes it, chapters included, from B.
    let book_id = imported.series_id.clone().unwrap();
    assert!(b.collection().unwrap().iter().any(|e| e.id == book_id));
    a.delete_entry(&book_id).await.unwrap();
    a.refresh().await.unwrap();
    b.refresh().await.unwrap();
    assert!(b.collection().unwrap().iter().all(|e| e.id != book_id));
    assert!(
        b.items(&ItemQuery {
            filter: ItemFilter::Series(book_id),
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );

    // Disconnecting keeps local data.
    b.disconnect_server().unwrap();
    assert_eq!(b.status().unwrap().mode, Mode::Unconfigured);
    assert_eq!(b.items(&ItemQuery::default()).unwrap().len(), 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_loads_history_in_the_background_and_announces_it() {
    let (fresh, _) = stateful_freshrss().await;
    let contents = "^/api/greader.php/reader/api/0/stream/contents/.*";
    let read_entry = |n: u64, title: &str| {
        json!({
            "id": format!("tag:google.com,2005:reader/item/{n:016x}"), "title": title, "published": 1_700_000_000 + n,
            "categories": ["user/-/state/com.google/reading-list", READ], "alternate": [{"href": format!("https://blog.example/{n}")}],
            "summary": {"content": format!("<p>Body of {title}.</p>")},
            "origin": {"streamId": "feed/1", "title": "Graphics Blog"}
        })
    };
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param_is_missing("c"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"items": [read_entry(1, "Old post")], "continuation": "older"}),
            ),
        )
        .with_priority(1)
        .mount(&fresh)
        .await;
    // Slow enough that the client has connected before the page lands.
    Mock::given(method("GET"))
        .and(path_regex(contents))
        .and(query_param("c", "older"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(1))
                .set_body_json(json!({"items": [read_entry(9, "Archived")]})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&fresh)
        .await;

    let server_dir = tempfile::tempdir().unwrap();
    let state = AppState::open(&ServerConfig {
        data_dir: server_dir.path().to_path_buf(),
        master_key: [3u8; 32],
    })
    .unwrap();
    state
        .engine
        .configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, router(state.clone())).into_future());

    let dir = tempfile::tempdir().unwrap();
    let device = client(dir.path());
    let code = create_pairing_code(&state.store, Duration::from_secs(60)).unwrap();
    device.connect_server(&url, &code, "Mac").await.unwrap();
    let has_archived = |engine: &Engine| {
        engine
            .items(&ItemQuery::default())
            .unwrap()
            .iter()
            .any(|r| r.item.title == "Archived")
    };
    assert!(!has_archived(&device), "history is not part of the refresh");

    device.start_live_sync();
    let watched = device.clone();
    assert!(
        eventually(move || has_archived(&watched)).await,
        "the server announced the history page"
    );
    assert!(find(&device, "Archived").item.read);
    device.stop_live_sync();
}

#[tokio::test]
async fn unauthenticated_and_tampered_requests_are_rejected() {
    let server_dir = tempfile::tempdir().unwrap();
    let state = AppState::open(&ServerConfig {
        data_dir: server_dir.path().to_path_buf(),
        master_key: [1u8; 32],
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, router(state.clone())).into_future());

    let anonymous = eddy_sync::SyncClient::new(&url, None).unwrap();
    assert!(matches!(
        anonymous.info().await,
        Err(eddy_sync::SyncError::Unauthorized)
    ));
    let forged = eddy_sync::SyncClient::new(&url, Some("f".repeat(64))).unwrap();
    assert!(matches!(
        forged.pull(0, 10).await,
        Err(eddy_sync::SyncError::Unauthorized)
    ));
    assert!(
        anonymous
            .pair(&eddy_sync::PairRequest {
                code: "WRONG-CODE".into(),
                device_id: "d".into(),
                device_name: "x".into()
            })
            .await
            .is_err()
    );

    let code = create_pairing_code(&state.store, Duration::from_secs(60)).unwrap();
    let paired = anonymous
        .pair(&eddy_sync::PairRequest {
            code: code.to_lowercase(),
            device_id: "d".into(),
            device_name: "x".into(),
        })
        .await
        .unwrap();
    let device = eddy_sync::SyncClient::new(&url, Some(paired.token)).unwrap();
    // A blob must match its hash.
    assert!(
        device
            .put_blob(&"0".repeat(64), b"content".to_vec(), "text/plain")
            .await
            .is_err()
    );
    let key = eddy_model::ids::sha256_hex(b"content");
    device
        .put_blob(&key, b"content".to_vec(), "text/plain")
        .await
        .unwrap();
    assert_eq!(device.get_blob(&key).await.unwrap().unwrap().0, b"content");
    assert!(device.put_config("../etc/passwd", "x").await.is_err());
}
