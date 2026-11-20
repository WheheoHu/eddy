//! Helpers shared by the server integration tests: a FreshRSS mock with
//! state, client engines in temporary directories, and a server that can be
//! stopped and started again on the same address.
#![allow(dead_code)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eddy_core::{Engine, EngineOptions, ItemQuery, MemoryPlatform};
use eddy_server::{AppState, ServerConfig, create_pairing_code, router};
use serde_json::json;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

pub const READ: &str = "user/-/state/com.google/read";
pub const STARRED: &str = "user/-/state/com.google/starred";
/// What the mock's FreshRSS calls its default category.
pub const DEFAULT_CATEGORY: &str = "未分类";

#[derive(Default)]
pub struct FreshState {
    pub listed: Vec<u64>,
    pub unread: HashSet<u64>,
    pub starred: HashSet<u64>,
    /// Feeds subscribed to through the API, after `feed/1`: the address
    /// asked for and the folder each was filed in.
    pub added: Vec<(String, Option<String>)>,
}

/// FreshRSS mock whose read/starred state changes with edit-tag calls.
pub async fn stateful_freshrss() -> (MockServer, Arc<Mutex<FreshState>>) {
    let server = MockServer::start().await;
    let state = Arc::new(Mutex::new(FreshState {
        listed: vec![1, 2, 3],
        unread: [2, 3].into(),
        starred: [1].into(),
        ..Default::default()
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
    let st = state.clone();
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/subscription/list"))
        .respond_with(move |_: &Request| {
            let s = st.lock().unwrap();
            let mut subscriptions = vec![json!({"id": "feed/1", "title": "Graphics Blog", "categories": [{"label": "Tech"}]})];
            for (n, (url, folder)) in s.added.iter().enumerate() {
                // No feed is in no category: the default one takes the rest,
                // named in the user's language.
                let categories = [json!({"label": folder.as_deref().unwrap_or(DEFAULT_CATEGORY)})];
                subscriptions.push(json!({
                    "id": format!("feed/{}", n + 2), "title": format!("Feed & {}", n + 2),
                    "categories": categories, "url": url, "htmlUrl": "https://added.example/"
                }));
            }
            ResponseTemplate::new(200).set_body_json(json!({ "subscriptions": subscriptions }))
        })
        .mount(&server)
        .await;
    // Subscribes like FreshRSS: the answer names the new stream, its title
    // still escaped; an address with no feed, or one subscribed to already,
    // is answered with why not.
    let st = state.clone();
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/subscription/quickadd"))
        .respond_with(move |req: &Request| {
            let url = url::form_urlencoded::parse(&req.body)
                .find(|(k, _)| k == "quickadd")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            let mut s = st.lock().unwrap();
            let refused = if url.contains("not-a-feed") {
                Some("Feed not found")
            } else if s.added.iter().any(|(known, _)| *known == url) {
                Some("Feed already exists")
            } else {
                None
            };
            if let Some(why) = refused {
                return ResponseTemplate::new(200).set_body_json(json!({"numResults": 0, "error": why}));
            }
            s.added.push((url.clone(), None));
            let n = s.added.len() + 1;
            ResponseTemplate::new(200).set_body_json(json!({
                "numResults": 1, "query": url, "streamId": format!("feed/{n}"), "streamName": format!("Feed &amp; {n}")
            }))
        })
        .mount(&server)
        .await;
    let st = state.clone();
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/subscription/edit"))
        .respond_with(move |req: &Request| {
            let form: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(&req.body)
                    .into_owned()
                    .collect();
            let feed = form
                .get("s")
                .and_then(|s| s.strip_prefix("feed/"))
                .and_then(|n| n.parse::<usize>().ok());
            let folder = form.get("a").and_then(|a| a.strip_prefix("user/-/label/"));
            let mut s = st.lock().unwrap();
            match (form.get("ac").map(String::as_str), feed, folder) {
                (Some("edit"), Some(n), Some(folder)) if n >= 2 && n - 2 < s.added.len() => {
                    s.added[n - 2].1 = Some(folder.to_string());
                    ResponseTemplate::new(200).set_body_string("OK")
                }
                _ => ResponseTemplate::new(400),
            }
        })
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

pub fn client(dir: &std::path::Path) -> Engine {
    Engine::open(
        dir,
        Arc::new(MemoryPlatform::default()),
        EngineOptions::default(),
    )
    .unwrap()
}

pub fn find(engine: &Engine, title: &str) -> eddy_core::ItemRow {
    let mut all = engine
        .items(&ItemQuery {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    all.retain(|r| r.item.title == title);
    all.pop().unwrap_or_else(|| panic!("{title} missing"))
}

pub async fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    false
}

/// `eventually` with a longer deadline, in seconds.
pub async fn eventually_for(seconds: u64, mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..seconds * 20 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// An eddy-server on a local port. `stop` closes it the way `docker stop`
/// does (connections included), so clients see it as unreachable; `start_at`
/// brings it back on the same address with the same data.
pub struct TestServer {
    pub state: Arc<AppState>,
    pub url: String,
    pub addr: SocketAddr,
    pub dir: PathBuf,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub async fn start(dir: &Path) -> TestServer {
        Self::start_at(dir, "127.0.0.1:0".parse().unwrap()).await
    }

    pub async fn start_at(dir: &Path, addr: SocketAddr) -> TestServer {
        Self::launch(dir, addr, None).await
    }

    /// With the admin page turned on.
    pub async fn start_with_admin(dir: &Path, password: &str) -> TestServer {
        Self::launch(dir, "127.0.0.1:0".parse().unwrap(), Some(password)).await
    }

    async fn launch(dir: &Path, addr: SocketAddr, admin: Option<&str>) -> TestServer {
        let state = AppState::open(&ServerConfig {
            data_dir: dir.to_path_buf(),
            master_key: [9u8; 32],
            admin_password: admin.map(String::from),
        })
        .unwrap();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let closing = state.clone();
        let app = router(state.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                    closing.closing.notify_waiters();
                })
                .await;
        });
        TestServer {
            state,
            url: format!("http://{addr}"),
            addr,
            dir: dir.to_path_buf(),
            stop,
            task,
        }
    }

    pub fn code(&self) -> String {
        create_pairing_code(&self.state.store, Duration::from_secs(60)).unwrap()
    }

    /// Stops the server and returns the address it was on.
    pub async fn stop(self) -> SocketAddr {
        let _ = self.stop.send(());
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
        self.addr
    }
}
