//! A device's first sync with a server that already holds a lot: pairing
//! does not wait for it, it says how far it is, and it arrives complete.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use eddy_core::{Engine, Mode, SyncPhase, SyncProgress};
use eddy_sync::{PROTOCOL_HEADER, PairRequest, SyncClient};

/// More than two pull pages (5000 each) and many push batches (500).
const TAGS: usize = 12_000;

/// Every progress state the engine reports, in order.
fn watch(engine: &Engine) -> Arc<Mutex<Vec<Option<SyncProgress>>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (log, watched) = (seen.clone(), engine.clone());
    engine.set_progress_listener(Some(Arc::new(move || {
        log.lock().unwrap().push(watched.status().unwrap().sync);
    })));
    seen
}

/// The reports of one phase: there are some, they never go backwards and
/// never pass their total.
fn assert_phase(seen: &[Option<SyncProgress>], phase: SyncPhase, total: u64) {
    let steps: Vec<SyncProgress> = seen
        .iter()
        .flatten()
        .filter(|p| p.phase == phase)
        .copied()
        .collect();
    assert!(!steps.is_empty(), "no {phase:?} progress in {seen:?}");
    assert!(steps.windows(2).all(|w| w[0].done <= w[1].done));
    for step in steps {
        assert_eq!(step.total, Some(total), "{phase:?}");
        assert!(step.done <= total);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_does_not_wait_for_the_first_sync_which_reports_progress() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;

    // A arrives with a long history of its own: the upload is shown.
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    for n in 0..TAGS {
        a.create_tag(&format!("tag {n}")).unwrap();
    }
    let seen_a = watch(&a);
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    assert_eq!(a.status().unwrap().pending_uploads, 0);
    assert_phase(&seen_a.lock().unwrap(), SyncPhase::Uploading, TAGS as u64);
    assert_eq!(a.status().unwrap().sync, None);
    let head = server.state.engine.server_cursor().unwrap();
    assert!(head >= TAGS as i64);

    // B is empty. Pairing connects it and returns; nothing has come down.
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    let seen_b = watch(&b);
    let changes = Arc::new(Mutex::new(0));
    let counted = changes.clone();
    b.set_change_listener(Some(Arc::new(move || *counted.lock().unwrap() += 1)));
    let status = b
        .pair_server(&server.url, &server.code(), "B")
        .await
        .unwrap();
    assert_eq!(status.cursor, 0);
    assert_eq!(b.status().unwrap().mode, Mode::Server);
    assert!(b.tags().unwrap().is_empty());

    // The sync runs in the background; a refresh asked for meanwhile (the
    // app's own timer, say) waits its turn instead of pulling alongside.
    b.start_sync();
    b.refresh().await.unwrap();
    let done = b.clone();
    assert!(eventually_for(30, move || done.status().unwrap().sync.is_none()).await);
    assert_eq!(b.tags().unwrap().len(), TAGS);
    assert_phase(&seen_b.lock().unwrap(), SyncPhase::Downloading, head as u64);
    assert_eq!(seen_b.lock().unwrap().last(), Some(&None), "progress ends");
    let told = changes.clone();
    assert!(
        eventually(move || *told.lock().unwrap() > 0).await,
        "lists are told to reload"
    );

    // The server knows where each device is.
    for device in [&a, &b] {
        let position = server.state.sync_log.position(&device.device_id()).unwrap();
        assert_eq!(position.cursor, head);
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pages_say_how_much_the_server_has_and_travel_compressed() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir = tempfile::tempdir().unwrap();
    let a = client(dir.path());
    for n in 0..50 {
        a.create_tag(&format!("tag {n}")).unwrap();
    }
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    let head = server.state.engine.server_cursor().unwrap();

    let token = SyncClient::new(&server.url, None)
        .unwrap()
        .pair(&PairRequest {
            code: server.code(),
            device_id: "raw".into(),
            device_name: "raw".into(),
        })
        .await
        .unwrap()
        .token;
    let page = SyncClient::new(&server.url, Some(token.clone()))
        .unwrap()
        .pull(0, 10)
        .await
        .unwrap();
    assert_eq!(page.ops.len(), 10);
    assert!(page.more);
    assert_eq!(page.head, Some(head));

    // Asked for gzip, the server sends gzip.
    let plain = reqwest::Client::builder().gzip(false).build().unwrap();
    let response = plain
        .get(format!("{}/v1/ops?since=0&limit=50", server.url))
        .bearer_auth(&token)
        .header(PROTOCOL_HEADER, eddy_sync::PROTOCOL_VERSION.to_string())
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["content-encoding"], "gzip");
    server.stop().await;
}

#[tokio::test]
async fn a_server_that_does_not_say_how_much_it_has_still_syncs() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let old = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ops"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ops": [], "cursor": 7, "more": false
        })))
        .mount(&old)
        .await;
    let page = SyncClient::new(&old.uri(), Some("token".into()))
        .unwrap()
        .pull(0, 10)
        .await
        .unwrap();
    assert_eq!((page.cursor, page.more, page.head), (7, false, None));
}
