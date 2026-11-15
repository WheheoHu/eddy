//! The admin page: off unless a password is set, closed to everything but a
//! signed-in browser session, and able to do what the command line does.

mod common;

use common::*;
use eddy_core::EngineError;
use eddy_server::admin::ADMIN_HEADER;
use eddy_sync::PROTOCOL_HEADER;
use reqwest::StatusCode;
use serde_json::{Value, json};

const PASSWORD: &str = "open sesame";

/// A browser: keeps the session cookie and sends the admin header.
struct Browser {
    http: reqwest::Client,
    base: String,
}

impl Browser {
    fn new(server: &TestServer) -> Browser {
        Browser {
            http: reqwest::Client::builder()
                .cookie_store(true)
                .build()
                .unwrap(),
            base: server.url.clone(),
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut req = self
            .http
            .request(method, format!("{}/admin/api/{path}", self.base))
            .header(ADMIN_HEADER, "1");
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.unwrap()
    }

    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        self.send(reqwest::Method::POST, path, Some(body)).await
    }

    async fn sign_in(&self) {
        let res = self.post("login", json!({ "password": PASSWORD })).await;
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    async fn state(&self) -> Value {
        let res = self.send(reqwest::Method::GET, "state", None).await;
        assert_eq!(res.status(), StatusCode::OK);
        res.json().await.unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_password_the_admin_page_does_not_exist() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path()).await;
    let browser = Browser::new(&server);
    let page = browser
        .http
        .get(format!("{}/admin", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    for (method, path) in [
        (reqwest::Method::GET, "state"),
        (reqwest::Method::POST, "login"),
        (reqwest::Method::POST, "pairing-codes"),
    ] {
        let res = browser
            .send(method, path, Some(json!({ "password": "" })))
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_signed_in_session_gets_in() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_admin(dir.path(), PASSWORD).await;
    let browser = Browser::new(&server);

    // The page itself is public (it is only the sign-in form) and locked down.
    let page = browser
        .http
        .get(format!("{}/admin", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()["x-frame-options"], "DENY");
    assert!(
        page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let html = page.text().await.unwrap();
    assert!(html.contains("Eddy Server"));
    assert!(!html.contains(PASSWORD));

    // Nothing works before signing in.
    let res = browser.send(reqwest::Method::GET, "state", None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = browser.post("pairing-codes", json!({})).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // A wrong password does not sign in.
    let res = browser.post("login", json!({ "password": "guess" })).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(res.headers().get("set-cookie").is_none());

    // A paired device's token is not an admin session, and the other way
    // round.
    let paired = eddy_sync::SyncClient::new(&server.url, None)
        .unwrap()
        .pair(&eddy_sync::PairRequest {
            code: server.code(),
            device_id: "dev".into(),
            device_name: "Device".into(),
        })
        .await
        .unwrap();
    let res = browser
        .http
        .get(format!("{}/admin/api/state", server.url))
        .bearer_auth(&paired.token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    browser.sign_in().await;
    let cookie_only = browser
        .http
        .get(format!("{}/v1/info", server.url))
        .header(PROTOCOL_HEADER, eddy_sync::PROTOCOL_VERSION)
        .send()
        .await
        .unwrap();
    assert_eq!(cookie_only.status(), StatusCode::UNAUTHORIZED);

    // Signed in, but a request another site could forge (no admin header)
    // changes nothing.
    let forged = browser
        .http
        .post(format!("{}/admin/api/pairing-codes", server.url))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let forged = browser
        .http
        .delete(format!("{}/admin/api/devices/dev", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    assert_eq!(server.state.store.devices().unwrap().len(), 1);

    // Signing out ends the session.
    let state = browser.state().await;
    assert_eq!(state["devices"][0]["name"], "Device");
    let res = browser.send(reqwest::Method::POST, "logout", None).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = browser.send(reqwest::Method::GET, "state", None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Guessing gets slower: after enough wrong passwords even the right
    // one has to wait.
    for _ in 0..5 {
        browser.post("login", json!({ "password": "guess" })).await;
    }
    let res = browser.post("login", json!({ "password": PASSWORD })).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn devices_are_paired_and_revoked_from_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_admin(dir.path(), PASSWORD).await;
    let browser = Browser::new(&server);
    browser.sign_in().await;

    let state = browser.state().await;
    assert_eq!(state["service"]["healthy"], true);
    assert!(
        state["service"]["version"]
            .as_str()
            .unwrap()
            .starts_with(eddy_core::CORE_VERSION)
    );
    assert_eq!(state["devices"].as_array().unwrap().len(), 0);
    assert_eq!(state["freshrss"]["account"], Value::Null);

    // A code made on the page pairs a real device, once.
    let made: Value = browser
        .post("pairing-codes", json!({ "minutes": 5 }))
        .await
        .json()
        .await
        .unwrap();
    let code = made["code"].as_str().unwrap().to_string();
    let left = made["expires_at"].as_i64().unwrap() - eddy_model::now_ms();
    assert!((4 * 60_000..=5 * 60_000).contains(&left));
    let dir_a = tempfile::tempdir().unwrap();
    let a = client(dir_a.path());
    a.connect_server(&server.url, &code, "Mac A").await.unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let b = client(dir_b.path());
    assert!(b.connect_server(&server.url, &code, "Mac B").await.is_err());

    a.start_live_sync();
    let hub = server.state.clone();
    assert!(eventually_for(10, move || hub.changes.receiver_count() == 1).await);
    let state = browser.state().await;
    assert_eq!(state["devices"][0]["name"], "Mac A");
    assert!(state["devices"][0]["last_seen"].is_i64());
    // It synced when it paired, so the page knows it has everything.
    assert_eq!(state["devices"][0]["behind"], 0);
    assert!(state["devices"][0]["synced_at"].is_i64());
    assert_eq!(state["service"]["live_connections"], 1);

    // Revoking stops the device and drops its open connection.
    let id = a.device_id();
    let res = browser
        .send(reqwest::Method::DELETE, &format!("devices/{id}"), None)
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(matches!(a.refresh().await, Err(EngineError::Unauthorized)));
    let hub = server.state.clone();
    assert!(
        eventually_for(10, move || hub.changes.receiver_count() == 0).await,
        "the revoked device's live connection was closed"
    );
    assert_eq!(
        browser.state().await["devices"].as_array().unwrap().len(),
        0
    );
    a.stop_live_sync();
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn freshrss_is_set_up_refreshed_and_watched_from_the_page() {
    let (fresh, fresh_state) = stateful_freshrss().await;
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_admin(dir.path(), PASSWORD).await;
    let browser = Browser::new(&server);
    browser.sign_in().await;

    // No account: nothing to refresh, and a wrong account is refused.
    let res = browser.send(reqwest::Method::POST, "refresh", None).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = browser
        .post(
            "account",
            json!({ "base_url": "http://127.0.0.1:1", "username": "me", "password": "secret-pw" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(!res.text().await.unwrap().contains("secret-pw"));

    let res = browser
        .post(
            "account",
            json!({ "base_url": fresh.uri(), "username": "me", "password": "secret-pw" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let mut state = Value::Null;
    for _ in 0..100 {
        state = browser.state().await;
        if state["freshrss"]["articles"] == 3 && state["freshrss"]["refreshing"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(state["freshrss"]["account"]["username"], "me");
    assert_eq!(state["freshrss"]["feeds"], 1);
    assert_eq!(state["freshrss"]["articles"], 3);
    assert!(state["freshrss"]["last_refresh"].is_i64());
    assert_eq!(state["freshrss"]["last_error"], Value::Null);
    assert!(
        !state.to_string().contains("secret-pw"),
        "the password never comes back"
    );

    // Refresh Now picks up a new article.
    fresh_state.lock().unwrap().listed.push(4);
    let res = browser.send(reqwest::Method::POST, "refresh", None).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    for _ in 0..100 {
        state = browser.state().await;
        if state["freshrss"]["articles"] == 4 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(state["freshrss"]["articles"], 4);

    // A failing FreshRSS (it now answers 404 to everything) shows up as
    // the last error.
    fresh.reset().await;
    browser.send(reqwest::Method::POST, "refresh", None).await;
    for _ in 0..100 {
        state = browser.state().await;
        if state["freshrss"]["last_error"].is_string() && state["freshrss"]["refreshing"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(state["freshrss"]["last_error"].is_string());

    // Video jobs: listed, and refused when the server is not set up for them.
    assert_eq!(state["video"]["capable"], false);
    assert_eq!(state["video"]["jobs"].as_array().unwrap().len(), 0);
    let res = browser
        .post(
            "video-jobs",
            json!({ "source": "https://example.com/talk" }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    server.stop().await;
}
