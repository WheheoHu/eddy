//! What happens around the happy path: the server goes away and comes back,
//! is restarted or replaced, a device is revoked, two devices disagree, a
//! change cannot be applied, and client and server versions drift apart.

mod common;

use common::*;
use eddy_core::{EngineError, ItemQuery, Mode};
use eddy_model::{Entity, Hlc, Op};
use eddy_sync::{PROTOCOL_HEADER, PairRequest, SyncClient};

fn count(engine: &eddy_core::Engine) -> usize {
    engine
        .items(&ItemQuery {
            limit: 1000,
            ..Default::default()
        })
        .unwrap()
        .len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_done_while_the_server_is_down_survives_and_converges() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;

    // A brings the FreshRSS account; B joins without one.
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    a.refresh().await.unwrap();
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    b.connect_server(&server.url, &server.code(), "B")
        .await
        .unwrap();
    let status = a.status().unwrap();
    assert!(status.server_online && status.pending_uploads == 0);
    assert!(status.offline_refresh, "A kept its FreshRSS account");
    assert!(!b.status().unwrap().offline_refresh);

    let addr = server.stop().await;

    // Everything local still works, and is queued.
    let unread = find(&a, "Unread elsewhere");
    a.set_starred(&unread.item.id, true).await.unwrap();
    a.set_read(&unread.item.id, true).await.unwrap();
    let tag = a.create_tag("Offline").unwrap();
    a.set_item_tag(&unread.item.id, &tag, true).unwrap();
    assert!(a.document(&unread.item.id).await.unwrap().is_some());

    // Refreshing brings new articles straight from FreshRSS.
    {
        let mut st = fresh_state.lock().unwrap();
        st.listed.push(4);
        st.unread.insert(4);
    }
    let summary = a.refresh().await.expect("offline refresh succeeds");
    assert!(summary.offline);
    assert_eq!(summary.new_items, 1);
    let arrived = find(&a, "Read on the web");
    assert!(!arrived.item.read);
    let status = a.status().unwrap();
    assert!(!status.server_online);
    assert!(status.pending_uploads >= 4, "changes wait for the server");
    assert_eq!(status.mode, Mode::Server);
    // ...without writing to FreshRSS behind the server's back.
    assert!(!fresh_state.lock().unwrap().starred.contains(&3));
    assert!(fresh_state.lock().unwrap().unread.contains(&3));

    // Search answers from this device, quickly.
    let started = std::time::Instant::now();
    let hits = a.search_all("radiance", 10).await.unwrap();
    assert!(!hits.is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));

    // A device without the account cannot refresh, and says why.
    assert!(matches!(b.refresh().await, Err(EngineError::Offline(_))));
    b.set_starred(&find(&b, "New post").item.id, true)
        .await
        .unwrap();

    // The server returns: both devices catch up and agree.
    let server = TestServer::start_at(server_dir.path(), addr).await;
    a.refresh().await.unwrap();
    let status = a.status().unwrap();
    assert!(status.server_online);
    assert_eq!(status.pending_uploads, 0);
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    assert_eq!(b.status().unwrap().pending_uploads, 0);
    assert_eq!(count(&a), 4);
    assert_eq!(count(&b), 4, "the article fetched offline is not doubled");
    let b_unread = find(&b, "Unread elsewhere");
    assert!(b_unread.item.starred && b_unread.item.read);
    assert_eq!(
        b.item_tags(&b_unread.item.id).unwrap().tags[0].name,
        "Offline"
    );
    assert!(find(&a, "New post").item.starred);
    assert_eq!(
        find(&b, "Read on the web").item.id,
        arrived.item.id,
        "same article, same ID, whichever side fetched it"
    );
    // The server, not the device, wrote the changes to FreshRSS.
    assert!(
        eventually(|| {
            let st = fresh_state.lock().unwrap();
            st.starred.contains(&3) && !st.unread.contains(&3) && st.starred.contains(&2)
        })
        .await
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_upload_on_their_own_once_the_server_is_back() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    let addr = server.stop().await;

    a.create_tag("Queued").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    assert_eq!(a.status().unwrap().pending_uploads, 1);
    assert!(!a.status().unwrap().server_online);

    // The live channel reconnects and pushes what is waiting, without a
    // refresh from the user.
    a.start_live_sync();
    let server = TestServer::start_at(server_dir.path(), addr).await;
    let engine = a.clone();
    assert!(
        eventually_for(20, move || {
            let s = engine.status().unwrap();
            s.pending_uploads == 0 && s.server_online
        })
        .await,
        "queued change reached the server"
    );
    assert!(
        server
            .state
            .engine
            .tags()
            .unwrap()
            .iter()
            .any(|t| t.name == "Queued")
    );

    // Starting and stopping repeatedly leaves one connection.
    for _ in 0..5 {
        a.stop_live_sync();
        a.start_live_sync();
        a.start_live_sync();
    }
    let state = server.state.clone();
    assert!(eventually_for(10, move || state.changes.receiver_count() == 1).await);
    a.stop_live_sync();
    let state = server.state.clone();
    assert!(eventually_for(10, move || state.changes.receiver_count() == 0).await);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_winning_flag_reaches_freshrss() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    b.connect_server(&server.url, &server.code(), "B")
        .await
        .unwrap();
    let addr = server.stop().await;

    // B stars first; A stars and takes it back later. A's last word wins,
    // even though B's older change arrives at the server after it.
    let id = find(&a, "New post").item.id;
    b.set_starred(&id, true).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    a.set_starred(&id, true).await.unwrap();
    a.set_starred(&id, false).await.unwrap();

    let server = TestServer::start_at(server_dir.path(), addr).await;
    a.refresh().await.unwrap();
    b.refresh().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    server.state.refresh().await.unwrap();
    a.refresh().await.unwrap();
    b.refresh().await.unwrap();
    assert!(
        !fresh_state.lock().unwrap().starred.contains(&2),
        "the older star never reached FreshRSS"
    );
    assert!(!find(&a, "New post").item.starred);
    assert!(!find(&b, "New post").item.starred);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_keeps_identity_devices_and_position() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    a.create_tag("Before").unwrap();
    a.refresh().await.unwrap();
    let before = a.server_status().await.unwrap().unwrap();
    assert!(!before.version.is_empty());

    let addr = server.stop().await;
    let server = TestServer::start_at(server_dir.path(), addr).await;
    a.create_tag("After").unwrap();
    a.refresh().await.expect("the device token still works");
    let after = a.server_status().await.unwrap().unwrap();
    assert_eq!(after.server_id, before.server_id);
    assert!(after.cursor > before.cursor);
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    b.connect_server(&server.url, &server.code(), "B")
        .await
        .unwrap();
    let mut names: Vec<String> = b.tags().unwrap().into_iter().map(|t| t.name).collect();
    names.sort();
    assert_eq!(names, ["After", "Before"]);

    // A revoked device is told to pair again, not that the server is down.
    let device = a.device_id();
    server.state.store.remove_device(&device).unwrap();
    assert!(matches!(a.refresh().await, Err(EngineError::Unauthorized)));
    assert!(a.status().unwrap().server_online);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_with_an_empty_server_uploads_the_history_again() {
    let first_dir = tempfile::tempdir().unwrap();
    let first = TestServer::start(first_dir.path()).await;
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.connect_server(&first.url, &first.code(), "A")
        .await
        .unwrap();
    a.create_tag("Kept").unwrap();
    a.refresh().await.unwrap();
    assert_eq!(a.status().unwrap().pending_uploads, 0);
    first.stop().await;

    // The test server is wiped and recreated: nothing of A's is on it.
    let second_dir = tempfile::tempdir().unwrap();
    let second = TestServer::start(second_dir.path()).await;
    a.connect_server(&second.url, &second.code(), "A")
        .await
        .unwrap();
    assert_eq!(a.status().unwrap().pending_uploads, 0);
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    b.connect_server(&second.url, &second.code(), "B")
        .await
        .unwrap();
    assert!(b.tags().unwrap().iter().any(|t| t.name == "Kept"));

    // Leaving server mode and pairing again works the same way.
    a.disconnect_server().unwrap();
    assert_eq!(a.status().unwrap().mode, Mode::Unconfigured);
    a.create_tag("Later").unwrap();
    a.connect_server(&second.url, &second.code(), "A")
        .await
        .unwrap();
    b.refresh().await.unwrap();
    assert!(b.tags().unwrap().iter().any(|t| t.name == "Later"));
    second.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_the_server_cannot_use_does_not_hold_up_the_rest() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let paired = SyncClient::new(&server.url, None)
        .unwrap()
        .pair(&PairRequest {
            code: server.code(),
            device_id: "raw".into(),
            device_name: "raw".into(),
        })
        .await
        .unwrap();
    let device = SyncClient::new(&server.url, Some(paired.token.clone())).unwrap();
    let now = eddy_model::now_ms() as u64;
    let op = |id: &str, wall_ms: u64, entity_id: &str, value: serde_json::Value| Op {
        op_id: id.into(),
        hlc: Hlc {
            wall_ms,
            counter: 0,
            node: "raw".into(),
        },
        entity: Entity::Tag,
        entity_id: entity_id.into(),
        field: "create".into(),
        value,
    };
    // One op from a clock two days ahead, one with a value of the wrong
    // type, two fine ones around them.
    let response = device
        .push(vec![
            op("op-1", now, "t1", serde_json::json!({"name": "One"})),
            op(
                "op-2",
                now + 48 * 3600 * 1000,
                "t2",
                serde_json::json!({"name": "Future"}),
            ),
            Op {
                field: "deleted".into(),
                ..op("op-3", now, "t1", serde_json::json!("not a boolean"))
            },
            op("op-4", now, "t4", serde_json::json!({"name": "Four"})),
        ])
        .await
        .unwrap();
    assert_eq!(response.applied, 2);
    let refused: Vec<&str> = response.rejected.iter().map(|r| r.op_id.as_str()).collect();
    assert_eq!(refused, ["op-2", "op-3"]);
    let names: Vec<String> = server
        .state
        .engine
        .tags()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names.contains(&"One".to_string()) && names.contains(&"Four".to_string()));
    assert!(!names.contains(&"Future".to_string()));

    // An entity only a newer client knows is refused on its own too.
    let http = reqwest::Client::new();
    let body: serde_json::Value = http
        .post(format!("{}/v1/ops", server.url))
        .bearer_auth(&paired.token)
        .header(PROTOCOL_HEADER, eddy_sync::PROTOCOL_VERSION)
        .json(&serde_json::json!({"ops": [
            {"op_id": "op-5", "hlc": format!("{now:013}-000001-raw"), "entity": "hologram",
             "entity_id": "h1", "field": "create", "value": {}},
            {"op_id": "op-6", "hlc": format!("{now:013}-000002-raw"), "entity": "tag",
             "entity_id": "t6", "field": "create", "value": {"name": "Six"}},
        ]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["applied"], 1);
    assert_eq!(body["rejected"][0]["op_id"], "op-5");

    // A real device carries on past a refused change of its own.
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    assert!(a.tags().unwrap().iter().any(|t| t.name == "Six"));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incompatible_versions_are_told_to_update() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let http = reqwest::Client::new();
    // Health checks carry no version and always work.
    let health = http
        .get(format!("{}/v1/health", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    // A client from before the protocol header, or an older protocol.
    for old in [None, Some("1")] {
        let mut req = http.get(format!("{}/v1/info", server.url));
        if let Some(v) = old {
            req = req.header(PROTOCOL_HEADER, v);
        }
        assert_eq!(req.send().await.unwrap().status(), 426);
    }
    // The current client gets in, and learns the server's version.
    let paired = SyncClient::new(&server.url, None)
        .unwrap()
        .pair(&PairRequest {
            code: server.code(),
            device_id: "d".into(),
            device_name: "d".into(),
        })
        .await
        .unwrap();
    assert_eq!(paired.info.protocol, eddy_sync::PROTOCOL_VERSION);
    assert!(paired.info.version.starts_with(eddy_core::CORE_VERSION));
    paired.info.check_protocol().unwrap();

    // A client refuses a server that is too old or too new for it.
    let mut info = paired.info.clone();
    info.protocol = eddy_sync::MIN_PROTOCOL - 1;
    assert!(matches!(
        info.check_protocol(),
        Err(eddy_sync::SyncError::Upgrade(_))
    ));
    let mut info = paired.info;
    info.min_protocol = eddy_sync::PROTOCOL_VERSION + 1;
    assert!(matches!(
        info.check_protocol(),
        Err(eddy_sync::SyncError::Upgrade(_))
    ));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_log_from_three_devices_arrives_complete() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dirs: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let mut devices = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        let d = client(dir.path());
        d.connect_server(&server.url, &server.code(), &format!("D{i}"))
            .await
            .unwrap();
        devices.push(d);
    }
    // More than one push batch (500) and one pull page (1000) in total,
    // pushed at the same time.
    for (i, d) in devices.iter().enumerate() {
        for n in 0..450 {
            d.create_tag(&format!("d{i}-{n}")).unwrap();
        }
    }
    let pushes: Vec<_> = devices
        .iter()
        .map(|d| {
            let d = d.clone();
            tokio::spawn(async move { d.refresh().await.unwrap() })
        })
        .collect();
    for p in pushes {
        p.await.unwrap();
    }
    for d in &devices {
        d.refresh().await.unwrap();
        assert_eq!(d.tags().unwrap().len(), 1350);
        assert_eq!(d.status().unwrap().pending_uploads, 0);
    }
    // A device that joins later pages through all of it.
    let late_dir = tempfile::tempdir().unwrap();
    let late = client(late_dir.path());
    late.connect_server(&server.url, &server.code(), "late")
        .await
        .unwrap();
    assert_eq!(late.tags().unwrap().len(), 1350);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removed_rule_pack_is_removed_everywhere() {
    let pack = "name = \"Private\"\nversion = 1\n[[site]]\ndomains = [\"example.com\"]\nstrategies = [\"archive\"]\n";
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.import_rule_pack("private", pack).unwrap();
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    b.connect_server(&server.url, &server.code(), "B")
        .await
        .unwrap();
    assert_eq!(b.rule_packs().packs.len(), 1);

    a.remove_rule_pack("private").unwrap();
    a.refresh().await.unwrap();
    assert!(a.rule_packs().packs.is_empty(), "it does not come back");
    b.refresh().await.unwrap();
    assert!(b.rule_packs().packs.is_empty(), "nor does B keep it");
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    assert!(a.rule_packs().packs.is_empty() && b.rule_packs().packs.is_empty());

    // Importing it again brings it back for both.
    b.import_rule_pack("private", pack).unwrap();
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    assert_eq!(a.rule_packs().packs.len(), 1);
    server.stop().await;
}
