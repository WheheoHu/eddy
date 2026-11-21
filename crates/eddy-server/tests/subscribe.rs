//! Subscribing to a feed from a device: the server is the one that asks
//! FreshRSS, every device gets the new source, and a device that cannot
//! reach the server is told so instead of writing to FreshRSS itself.

mod common;

use common::*;
use eddy_core::{Engine, EngineError};
use eddy_sync::{AddFeedRequest, SyncClient, SyncError};

fn folder_of(engine: &Engine, source_id: &str) -> Option<Option<String>> {
    engine
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.source.id == source_id)
        .map(|s| s.source.category)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_feed_added_on_one_device_is_subscribed_by_the_server_for_all() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;

    // A brings the FreshRSS account and keeps it; B has none.
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

    // B, which could not reach FreshRSS if it tried, adds a feed to a
    // folder that does not exist yet.
    let added = b
        .add_feed(" https://news.example/feed.xml ", Some("  Reading "))
        .await
        .unwrap();
    assert_eq!(added.title, "Feed & 2", "as FreshRSS lists it, unescaped");
    assert_eq!(added.folder.as_deref(), Some("Reading"));
    assert_eq!(
        fresh_state.lock().unwrap().added,
        [(
            "https://news.example/feed.xml".to_string(),
            Some("Reading".to_string())
        )]
    );
    // It reaches both devices as the same source.
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    for device in [&a, &b] {
        assert_eq!(
            folder_of(device, &added.source_id),
            Some(Some("Reading".to_string()))
        );
    }

    // Without a folder the feed is filed in none: FreshRSS puts it in its
    // default category, which is not a folder here, whatever it is called.
    let loose = a.add_feed("https://loose.example/rss", None).await.unwrap();
    assert_eq!(loose.folder, None);
    assert_eq!(fresh_state.lock().unwrap().added[1].1, None);
    b.refresh().await.unwrap();
    a.refresh().await.unwrap();
    for device in [&a, &b] {
        assert_eq!(folder_of(device, &loose.source_id), Some(None));
    }
    // It stays that way through the server's own refreshes.
    server.state.refresh().await.unwrap();
    b.refresh().await.unwrap();
    assert_eq!(folder_of(&b, &loose.source_id), Some(None));

    // FreshRSS's own reasons come back as they are, and nothing is added.
    for (url, why) in [
        ("https://news.example/feed.xml", "Feed already exists"),
        ("https://blog.example/not-a-feed", "Feed not found"),
    ] {
        match b.add_feed(url, None).await {
            Err(EngineError::Invalid(message)) => assert!(message.contains(why), "{message}"),
            other => panic!("{url}: {other:?}"),
        }
    }
    assert!(matches!(
        b.add_feed("not an address", None).await,
        Err(EngineError::Invalid(_))
    ));
    assert_eq!(fresh_state.lock().unwrap().added.len(), 2);

    // With the server away a device says so. A has the account on board
    // and still leaves FreshRSS alone: only the server writes to it.
    server.stop().await;
    for device in [&a, &b] {
        assert!(matches!(
            device.add_feed("https://later.example/feed", None).await,
            Err(EngineError::Offline(_))
        ));
        assert!(!device.status().unwrap().server_online);
    }
    assert_eq!(fresh_state.lock().unwrap().added.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adding_a_feed_needs_a_paired_device_and_an_account() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let request = AddFeedRequest {
        url: "https://news.example/feed.xml".into(),
        folder: None,
    };

    // Not paired.
    let stranger = SyncClient::new(&server.url, Some("nope".into())).unwrap();
    assert!(matches!(
        stranger.add_feed(&request).await,
        Err(SyncError::Unauthorized)
    ));

    // Paired, with a server that has no FreshRSS account yet.
    let dir = tempfile::tempdir().unwrap();
    let device = client(dir.path());
    device
        .connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    assert!(matches!(
        device.add_feed(&request.url, None).await,
        Err(EngineError::Invalid(_))
    ));

    // A device with neither a server nor FreshRSS.
    let alone_dir = tempfile::tempdir().unwrap();
    let alone = client(alone_dir.path());
    assert!(matches!(
        alone.add_feed(&request.url, None).await,
        Err(EngineError::NotConfigured)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standalone_device_subscribes_with_freshrss_itself() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let device = client(dir.path());
    device
        .configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    device.refresh().await.unwrap();

    let added = device
        .add_feed("news.example/feed.xml", Some("Tech"))
        .await
        .unwrap();
    // Listed at once, before any refresh, and still there after one.
    assert_eq!(
        folder_of(&device, &added.source_id),
        Some(Some("Tech".to_string()))
    );
    assert_eq!(
        fresh_state.lock().unwrap().added,
        [(
            "https://news.example/feed.xml".to_string(),
            Some("Tech".to_string())
        )]
    );
    device.refresh().await.unwrap();
    assert_eq!(
        folder_of(&device, &added.source_id),
        Some(Some("Tech".to_string()))
    );
    assert!(matches!(
        device
            .add_feed("https://blog.example/not-a-feed", None)
            .await,
        Err(EngineError::Invalid(_))
    ));

    // One added without a folder is in none, now and after a refresh.
    let loose = device
        .add_feed("https://loose.example/rss", None)
        .await
        .unwrap();
    assert_eq!(loose.folder, None);
    device.refresh().await.unwrap();
    assert_eq!(folder_of(&device, &loose.source_id), Some(None));
}
