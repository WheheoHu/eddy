//! Signing a device out and erasing it: what was only on the device is
//! sent first, the server forgets the device, and the next open starts
//! from nothing.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::*;
use eddy_core::{
    Engine, EngineError, EngineOptions, ItemQuery, MemoryPlatform, Mode, Platform, SignOut,
};

/// A device whose secrets the test can look at, and reopen with.
fn open(dir: &Path, platform: &Arc<MemoryPlatform>) -> Engine {
    Engine::open(dir, platform.clone(), EngineOptions::default()).unwrap()
}

fn count(engine: &Engine) -> usize {
    engine
        .items(&ItemQuery {
            limit: 1000,
            ..Default::default()
        })
        .unwrap()
        .len()
}

fn erase_is_pending(dir: &Path) -> bool {
    dir.join("erase-on-open").exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signing_out_sends_what_is_left_leaves_the_server_and_erases() {
    let (fresh, _) = stateful_freshrss().await;
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;

    let dir_a = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemoryPlatform::default());
    let a = open(dir_a.path(), &secrets);
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
    assert!(count(&a) > 0);
    let device = a.device_id();
    // Something that is not Eddy's sits in the same directory.
    std::fs::write(dir_a.path().join("notes.txt"), "mine").unwrap();
    // A change made just before leaving, not uploaded yet.
    a.create_tag("Last minute").unwrap();

    assert_eq!(a.sign_out_and_erase(false).await.unwrap(), SignOut::Erased);
    assert!(erase_is_pending(dir_a.path()));
    assert_eq!(a.status().unwrap().mode, Mode::Unconfigured);
    assert!(secrets.secret_get("server:token").is_none());

    // The change reached the server, which no longer knows the device.
    b.refresh().await.unwrap();
    assert!(b.tags().unwrap().iter().any(|t| t.name == "Last minute"));
    let devices = server.state.store.devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "B");

    // The next open starts from nothing, as a new device, and leaves
    // what is not Eddy's alone.
    drop(a);
    let again = open(dir_a.path(), &secrets);
    assert!(!erase_is_pending(dir_a.path()));
    assert_eq!(again.status().unwrap().mode, Mode::Unconfigured);
    assert_eq!(again.status().unwrap().account, None);
    assert_eq!(count(&again), 0);
    assert!(again.tags().unwrap().is_empty());
    assert_ne!(again.device_id(), device);
    assert_eq!(
        std::fs::read_to_string(dir_a.path().join("notes.txt")).unwrap(),
        "mine"
    );

    // Pairing again brings the library back.
    again
        .connect_server(&server.url, &server.code(), "A again")
        .await
        .unwrap();
    assert!(count(&again) > 0);
    assert!(
        again
            .tags()
            .unwrap()
            .iter()
            .any(|t| t.name == "Last minute")
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_that_cannot_be_sent_are_only_lost_when_that_is_accepted() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemoryPlatform::default());
    let a = open(dir.path(), &secrets);
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    server.stop().await;
    a.create_tag("Offline").unwrap();

    // Nothing happens while the change has nowhere to go.
    assert_eq!(
        a.sign_out_and_erase(false).await.unwrap(),
        SignOut::Unsent(1)
    );
    assert!(!erase_is_pending(dir.path()));
    assert_eq!(a.status().unwrap().mode, Mode::Server);
    assert!(secrets.secret_get("server:token").is_some());
    assert_eq!(a.tags().unwrap().len(), 1);

    // Accepting the loss signs out even though the server cannot be told.
    assert_eq!(a.sign_out_and_erase(true).await.unwrap(), SignOut::Erased);
    assert!(secrets.secret_get("server:token").is_none());
    drop(a);
    let again = open(dir.path(), &secrets);
    assert_eq!(again.status().unwrap().mode, Mode::Unconfigured);
    assert!(again.tags().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standalone_device_signs_out_of_freshrss_and_erases() {
    let (fresh, _) = stateful_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemoryPlatform::default());
    let a = open(dir.path(), &secrets);
    let account = a
        .configure_freshrss(&fresh.uri(), "me", "pw")
        .await
        .unwrap();
    a.refresh().await.unwrap();
    a.create_tag("Only here").unwrap();
    let password = format!("account:{}", account.id);
    assert!(secrets.secret_get(&password).is_some());

    assert_eq!(a.sign_out_and_erase(false).await.unwrap(), SignOut::Erased);
    assert!(secrets.secret_get(&password).is_none());
    assert!(matches!(a.refresh().await, Err(EngineError::NotConfigured)));
    drop(a);
    let again = open(dir.path(), &secrets);
    assert_eq!(again.status().unwrap().mode, Mode::Unconfigured);
    assert_eq!(count(&again), 0);
    assert!(again.tags().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disconnecting_keeps_the_data_and_leaves_the_server() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(server_dir.path()).await;
    let dir = tempfile::tempdir().unwrap();
    let a = client(dir.path());
    a.connect_server(&server.url, &server.code(), "A")
        .await
        .unwrap();
    a.create_tag("Kept").unwrap();
    a.disconnect_server().await.unwrap();
    assert_eq!(a.tags().unwrap().len(), 1);
    assert!(server.state.store.devices().unwrap().is_empty());
    server.stop().await;
}
