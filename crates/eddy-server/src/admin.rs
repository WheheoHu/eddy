//! The admin page at `/admin`: pairing codes, devices, FreshRSS and video
//! jobs, for whoever runs the server.
//!
//! It exists only when `EDDY_ADMIN_PASSWORD` is set. A pairing code admits a
//! new device to everything the server holds, so the page is behind that
//! password and nothing else: a device token does not open it, and an admin
//! session does not open the sync API.
//!
//! Sessions are random tokens kept in memory (a restart signs everyone out)
//! in an `HttpOnly`, `SameSite=Strict` cookie. Requests that change anything
//! must also carry the `x-eddy-admin` header, which a page on another site
//! cannot add.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use eddy_model::ids::sha256_hex;
use eddy_model::now_ms;
use eddy_sync::{AccountRequest, CreateVideoJobRequest, RemoteVideoJob};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::{ApiError, AppState, create_pairing_code, secrets};

const PAGE: &str = include_str!("../assets/admin.html");
const COOKIE: &str = "eddy_admin";
const SESSION_TTL: Duration = Duration::from_secs(12 * 3600);
/// Header every state-changing request must carry.
pub const ADMIN_HEADER: &str = "x-eddy-admin";
/// Wrong passwords allowed before each further try has to wait.
const FREE_ATTEMPTS: u32 = 5;
const MAX_LOCKOUT: Duration = Duration::from_secs(300);
/// Longest a pairing code made here may live.
const MAX_CODE_MINUTES: u64 = 24 * 60;

pub struct Admin {
    /// Hash of the password, so comparing never depends on its length.
    password_hash: String,
    /// Hash of each session token -> when it expires.
    sessions: Mutex<HashMap<String, Instant>>,
    /// Consecutive wrong passwords, and when the next try is allowed.
    failures: Mutex<(u32, Option<Instant>)>,
}

impl Admin {
    pub fn new(password: &str) -> Admin {
        Admin {
            password_hash: sha256_hex(password.as_bytes()),
            sessions: Mutex::new(HashMap::new()),
            failures: Mutex::new((0, None)),
        }
    }

    /// Checks the password. `Err` carries how long to wait when too many
    /// wrong ones came in a row; the wait doubles each time, for everyone,
    /// since behind a proxy all requests look like one client.
    fn login(&self, password: &str) -> Result<Option<String>, Duration> {
        let mut failures = self.failures.lock();
        if let Some(until) = failures.1
            && let Some(wait) = until.checked_duration_since(Instant::now())
        {
            return Err(wait);
        }
        if !same(&sha256_hex(password.as_bytes()), &self.password_hash) {
            failures.0 += 1;
            if failures.0 >= FREE_ATTEMPTS {
                let doubling = (failures.0 - FREE_ATTEMPTS).min(16);
                let wait = Duration::from_secs(2u64.pow(doubling)).min(MAX_LOCKOUT);
                failures.1 = Some(Instant::now() + wait);
            }
            return Ok(None);
        }
        *failures = (0, None);
        let token = secrets::new_token();
        let mut sessions = self.sessions.lock();
        sessions.retain(|_, expires| *expires > Instant::now());
        sessions.insert(sha256_hex(token.as_bytes()), Instant::now() + SESSION_TTL);
        Ok(Some(token))
    }

    fn signed_in(&self, token: &str) -> bool {
        self.sessions
            .lock()
            .get(&sha256_hex(token.as_bytes()))
            .is_some_and(|expires| *expires > Instant::now())
    }

    fn logout(&self, token: &str) {
        self.sessions.lock().remove(&sha256_hex(token.as_bytes()));
    }
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_string())
}

fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Not found".into())
}

/// A signed-in admin request. Without `EDDY_ADMIN_PASSWORD` every admin
/// route answers 404, as if it were not there.
pub struct Session(String);

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let admin = state.admin.as_ref().ok_or_else(not_found)?;
        if parts.method != Method::GET && !parts.headers.contains_key(ADMIN_HEADER) {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "This request has to come from the admin page.".into(),
            ));
        }
        let token = session_cookie(&parts.headers)
            .filter(|t| admin.signed_in(t))
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
        Ok(Session(token))
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin", get(page))
        .route("/admin/", get(page))
        .route("/admin/api/login", post(login))
        .route("/admin/api/logout", post(logout))
        .route("/admin/api/state", get(state))
        .route("/admin/api/pairing-codes", post(pairing_code))
        .route("/admin/api/devices/{id}", delete(revoke_device))
        .route("/admin/api/refresh", post(refresh))
        .route("/admin/api/account", post(set_account))
        .route("/admin/api/video-jobs", post(create_video_job))
        .route("/admin/api/video-jobs/{id}/retry", post(retry_video_job))
        .route("/admin/api/video-jobs/{id}/cancel", post(cancel_video_job))
        .route("/admin/api/video-jobs/{id}", delete(dismiss_video_job))
}

async fn page(State(s): State<Arc<AppState>>) -> Result<Response, ApiError> {
    if s.admin.is_none() {
        return Err(not_found());
    }
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
        ],
        Html(PAGE),
    )
        .into_response())
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

async fn login(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let admin = s.admin.as_ref().ok_or_else(not_found)?;
    if !headers.contains_key(ADMIN_HEADER) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "This request has to come from the admin page.".into(),
        ));
    }
    match admin.login(&req.password) {
        Ok(Some(token)) => {
            // Behind an HTTPS proxy the cookie must not travel over HTTP.
            let secure = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("https"));
            let cookie = format!(
                "{COOKIE}={token}; Path=/admin; HttpOnly; SameSite=Strict; Max-Age={}{}",
                SESSION_TTL.as_secs(),
                if secure { "; Secure" } else { "" }
            );
            tracing::info!("admin signed in");
            Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response())
        }
        Ok(None) => {
            tracing::warn!("admin sign-in failed: wrong password");
            Err(ApiError(StatusCode::UNAUTHORIZED, "Wrong password.".into()))
        }
        Err(wait) => Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many wrong passwords. Try again in {} seconds.",
                wait.as_secs().max(1)
            ),
        )),
    }
}

async fn logout(State(s): State<Arc<AppState>>, Session(token): Session) -> Response {
    if let Some(admin) = &s.admin {
        admin.logout(&token);
    }
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE}=; Path=/admin; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

#[derive(Serialize)]
struct ServiceView {
    version: String,
    protocol: u32,
    min_protocol: u32,
    server_id: String,
    started_at: i64,
    now: i64,
    healthy: bool,
    cursor: i64,
    /// Bytes on disk: database, content, search index.
    database_bytes: u64,
    content_bytes: u64,
    index_bytes: u64,
    /// Devices with a live connection open right now.
    live_connections: usize,
}

#[derive(Serialize)]
struct DeviceView {
    id: String,
    name: String,
    created_at: i64,
    last_seen: Option<i64>,
}

#[derive(Serialize)]
struct FreshRssView {
    account: Option<AccountView>,
    refreshing: bool,
    loading_history: bool,
    last_refresh: Option<i64>,
    last_error: Option<String>,
    /// Seconds between scheduled refreshes.
    interval: u64,
    feeds: u32,
    articles: u32,
}

#[derive(Serialize)]
struct AccountView {
    base_url: String,
    username: String,
}

#[derive(Serialize)]
struct VideoView {
    /// Models and tools are configured: jobs can run here.
    capable: bool,
    jobs: Vec<RemoteVideoJob>,
}

#[derive(Serialize)]
struct StateView {
    service: ServiceView,
    devices: Vec<DeviceView>,
    freshrss: FreshRssView,
    video: VideoView,
}

fn dir_bytes(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

async fn state(State(s): State<Arc<AppState>>, _a: Session) -> Result<Json<StateView>, ApiError> {
    let overview = s.engine.server_overview().map_err(ApiError::internal)?;
    let database_bytes = ["eddy.sqlite3", "eddy.sqlite3-wal"]
        .iter()
        .filter_map(|name| std::fs::metadata(s.data_dir.join(name)).ok())
        .map(|m| m.len())
        .sum();
    Ok(Json(StateView {
        service: ServiceView {
            version: eddy_core::core_version(),
            protocol: eddy_sync::PROTOCOL_VERSION,
            min_protocol: eddy_sync::MIN_PROTOCOL,
            server_id: s.server_id.clone(),
            started_at: s.started_at,
            now: now_ms(),
            healthy: s.engine.server_healthy(),
            cursor: s.engine.server_cursor().map_err(ApiError::internal)?,
            database_bytes,
            content_bytes: overview.content_bytes,
            index_bytes: dir_bytes(&s.data_dir.join("index")),
            live_connections: s.changes.receiver_count(),
        },
        devices: s
            .store
            .devices()
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|d| DeviceView {
                id: d.id,
                name: d.name,
                created_at: d.created_at,
                last_seen: d.last_seen,
            })
            .collect(),
        freshrss: FreshRssView {
            account: s.engine.server_account().map(|a| AccountView {
                base_url: a.base_url,
                username: a.username,
            }),
            refreshing: s.refreshing.load(Ordering::SeqCst),
            loading_history: overview.loading_history,
            last_refresh: overview.last_refresh,
            last_error: overview.last_error,
            interval: s.refresh_interval.load(Ordering::SeqCst),
            feeds: overview.feeds,
            articles: overview.articles,
        },
        video: VideoView {
            capable: s.engine.server_video_capable(),
            jobs: s.video_jobs()?,
        },
    }))
}

#[derive(Deserialize)]
struct PairingCodeRequest {
    #[serde(default = "default_code_minutes")]
    minutes: u64,
}

fn default_code_minutes() -> u64 {
    15
}

#[derive(Serialize)]
struct PairingCodeView {
    code: String,
    expires_at: i64,
}

async fn pairing_code(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Json(req): Json<PairingCodeRequest>,
) -> Result<Json<PairingCodeView>, ApiError> {
    let minutes = req.minutes.clamp(1, MAX_CODE_MINUTES);
    let code = create_pairing_code(&s.store, Duration::from_secs(minutes * 60))
        .map_err(ApiError::internal)?;
    tracing::info!("admin created a pairing code valid for {minutes} minutes");
    Ok(Json(PairingCodeView {
        code,
        expires_at: now_ms() + (minutes * 60_000) as i64,
    }))
}

async fn revoke_device(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.revoke_device(&id).map_err(ApiError::internal)?;
    tracing::info!("admin revoked device {id}");
    Ok(StatusCode::NO_CONTENT)
}

/// Starts a refresh and returns at once; the page watches `refreshing`.
async fn refresh(State(s): State<Arc<AppState>>, _a: Session) -> Result<StatusCode, ApiError> {
    if s.engine.server_account().is_none() {
        return Err(ApiError::bad("Set a FreshRSS account first."));
    }
    if !s.refreshing.load(Ordering::SeqCst) {
        let state = s.clone();
        tokio::spawn(async move {
            if let Err(e) = state.refresh().await {
                tracing::warn!("refresh from the admin page failed: {e}");
            }
        });
    }
    Ok(StatusCode::ACCEPTED)
}

async fn set_account(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Json(req): Json<AccountRequest>,
) -> Result<StatusCode, ApiError> {
    s.engine
        .configure_freshrss(&req.base_url, &req.username, &req.password)
        .await
        .map_err(ApiError::bad)?;
    tracing::info!("admin set the FreshRSS account");
    let state = s.clone();
    tokio::spawn(async move {
        if let Err(e) = state.refresh().await {
            tracing::warn!("first refresh of the new account failed: {e}");
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

async fn create_video_job(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Json(req): Json<CreateVideoJobRequest>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    if !s.engine.server_video_capable() {
        return Err(ApiError::bad(
            "Video jobs are not set up on this server (EDDY_VIDEO_* in .env).",
        ));
    }
    Ok(Json(s.create_video_job(req)?))
}

async fn retry_video_job(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    Ok(Json(s.retry_video_job(&id)?))
}

async fn cancel_video_job(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    Ok(Json(s.cancel_video_job(&id)?))
}

async fn dismiss_video_job(
    State(s): State<Arc<AppState>>,
    _a: Session,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.dismiss_video_job(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_passwords_slow_down_and_the_right_one_resets() {
        let admin = Admin::new("correct horse");
        for _ in 0..FREE_ATTEMPTS - 1 {
            assert!(matches!(admin.login("nope"), Ok(None)));
        }
        // The fifth wrong one starts the wait; even the right password has
        // to sit it out.
        assert!(matches!(admin.login("nope"), Ok(None)));
        assert!(admin.login("correct horse").is_err());
        admin.failures.lock().1 = None;
        let token = admin.login("correct horse").unwrap().unwrap();
        assert!(admin.signed_in(&token));
        assert!(!admin.signed_in("something else"));
        assert_eq!(*admin.failures.lock(), (0, None));
        admin.logout(&token);
        assert!(!admin.signed_in(&token));
    }

    #[test]
    fn the_session_cookie_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "a=1; eddy_admin=tok; b=2".parse().unwrap());
        assert_eq!(session_cookie(&headers).as_deref(), Some("tok"));
        headers.insert(header::COOKIE, "eddy_admin_x=no".parse().unwrap());
        assert_eq!(session_cookie(&headers), None);
    }
}
