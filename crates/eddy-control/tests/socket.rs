//! The control socket between the command line and the app.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eddy_control::dto::{AddedDto, HelloDto, ItemPage};
use eddy_control::{
    CONTROL_VERSION, Frame, POINTER_NAME, Request, SOCKET_NAME, Scope, call, serve, socket_path,
};
use eddy_core::{Engine, EngineOptions, MemoryPlatform};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn open(dir: &std::path::Path) -> Engine {
    Engine::open(
        dir,
        Arc::new(MemoryPlatform::default()),
        EngineOptions::default(),
    )
    .unwrap()
}

/// A directory with a path short enough for a socket, whatever the
/// session's temporary directory is.
fn short_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("eddy-ct-")
        .tempdir_in("/tmp")
        .unwrap()
}

fn add_text() -> Request {
    Request::AddText {
        title: Some("Note".into()),
        body: "Some words.".into(),
        html: false,
        url: None,
    }
}

fn saved() -> Request {
    Request::ItemsList {
        scope: Scope::Saved,
        unread: false,
        limit: 10,
        offset: 0,
    }
}

/// One raw line sent to the socket, and the frame that answers it.
async fn raw(socket: &std::path::Path, line: &str) -> Frame {
    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (read, mut write) = stream.into_split();
    write.write_all(line.as_bytes()).await.unwrap();
    let answer = BufReader::new(read).lines().next_line().await.unwrap();
    serde_json::from_str(&answer.unwrap()).unwrap()
}

// The app's runtime has one thread, like this test's: requests are carried
// out off it and must still get their I/O driven.
#[tokio::test]
async fn requests_are_answered_and_changes_announced() {
    let dir = short_dir();
    let engine = open(dir.path());
    let changes = Arc::new(AtomicU32::new(0));
    let seen = changes.clone();
    engine.set_change_listener(Some(Arc::new(move || {
        seen.fetch_add(1, Ordering::SeqCst);
    })));

    let server = serve(engine.clone(), dir.path()).unwrap();
    let socket = socket_path(dir.path());
    assert_eq!(socket, dir.path().join(SOCKET_NAME));
    assert_eq!(server.socket(), socket);
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "only the user can open the socket");

    let hello: HelloDto =
        serde_json::from_value(call(&socket, &Request::Hello, |_| {}).await.unwrap()).unwrap();
    assert_eq!(hello.control, CONTROL_VERSION);
    assert_eq!(hello.pid, std::process::id());
    assert_eq!(changes.load(Ordering::SeqCst), 0, "reading changes nothing");

    let added: AddedDto =
        serde_json::from_value(call(&socket, &add_text(), |_| {}).await.unwrap()).unwrap();
    assert_eq!(
        changes.load(Ordering::SeqCst),
        1,
        "open lists are told to reload"
    );
    let page: ItemPage =
        serde_json::from_value(call(&socket, &saved(), |_| {}).await.unwrap()).unwrap();
    assert_eq!(page.items[0].id, added.id);
    assert!(engine.item(&added.id).unwrap().is_some());

    // Engine errors come through with their name.
    let refused = call(&socket, &Request::Sync { history: false }, |_| {})
        .await
        .unwrap_err();
    assert_eq!(refused.code, "not_configured");

    // A command line from another version is told to restart the app.
    let other = raw(
        &socket,
        "{\"v\":99,\"core\":\"9.9.9 (abc)\",\"request\":{\"cmd\":\"from the future\"}}\n",
    )
    .await;
    let Frame::Error(mismatch) = other else {
        panic!("{other:?}")
    };
    assert_eq!(mismatch.code, "version_mismatch");
    let Frame::Error(garbled) = raw(&socket, "not json\n").await else {
        panic!()
    };
    assert_eq!(garbled.code, "bad_request");

    // Stopping removes the socket, and callers hear that nobody is there.
    drop(server);
    assert!(!socket.exists());
    let nobody = call(&socket, &Request::Hello, |_| {}).await.unwrap_err();
    assert_eq!(nobody.code, "app_unreachable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_left_behind_is_replaced_and_requests_run_side_by_side() {
    let dir = short_dir();
    // What a crashed app leaves: a socket file nobody listens on.
    drop(std::os::unix::net::UnixListener::bind(dir.path().join(SOCKET_NAME)).unwrap());
    assert!(dir.path().join(SOCKET_NAME).exists());
    let engine = open(dir.path());
    let _server = serve(engine, dir.path()).unwrap();
    let socket = socket_path(dir.path());

    let adds = (0..8).map(|_| {
        let socket = socket.clone();
        tokio::spawn(async move { call(&socket, &add_text(), |_| {}).await })
    });
    for add in adds.collect::<Vec<_>>() {
        add.await.unwrap().unwrap();
    }
    let page: ItemPage =
        serde_json::from_value(call(&socket, &saved(), |_| {}).await.unwrap()).unwrap();
    assert_eq!(page.total, 8);
}

#[tokio::test]
async fn a_data_directory_too_deep_for_a_socket_points_to_one_elsewhere() {
    let root = short_dir();
    let deep = root.path().join("a".repeat(60)).join("b".repeat(60));
    std::fs::create_dir_all(&deep).unwrap();
    let engine = open(&deep);
    let server = serve(engine, &deep).unwrap();

    let socket = socket_path(&deep);
    assert_ne!(socket, deep.join(SOCKET_NAME));
    assert!(socket.as_os_str().len() < 100, "{}", socket.display());
    assert_eq!(
        std::fs::read_to_string(deep.join(POINTER_NAME)).unwrap(),
        socket.to_str().unwrap()
    );
    call(&socket, &Request::Hello, |_| {}).await.unwrap();

    drop(server);
    assert!(!socket.exists());
    assert!(!deep.join(POINTER_NAME).exists());
    assert_eq!(socket_path(&deep), deep.join(SOCKET_NAME));
}
