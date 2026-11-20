//! eddy-server: optional sync hub for Eddy clients. It owns the FreshRSS
//! account, keeps the op log every device syncs with, stores content blobs,
//! distributes private configuration and searches the full history.

pub mod admin;
pub mod progress;
pub mod secrets;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use eddy_core::{
    Engine, EngineOptions, HistoryCallback, StartVideoJobOptions, TranscriptPolicy, VideoJobTarget,
    VideoPipelineMode, record_to_remote,
};
use eddy_model::ids::sha256_hex;
use eddy_store::Store;
use eddy_sync::{
    AccountInfo, AccountRequest, ConfigEntry, CreateVideoJobRequest, ErrorBody, MIN_PROTOCOL,
    PROTOCOL_HEADER, PROTOCOL_VERSION, PairRequest, PairResponse, PullResponse, PushResponse,
    RawPushRequest, RejectedOp, RemoteVideoJob, SearchHit, ServerInfo,
};
use serde::Deserialize;
use tokio::sync::broadcast;

pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub master_key: [u8; 32],
    /// Turns on the admin page at `/admin`; without it the page does not
    /// exist.
    pub admin_password: Option<String>,
}

pub struct AppState {
    pub engine: Engine,
    /// Separate handle for devices, pairing codes and config.
    pub store: Store,
    pub server_id: String,
    pub changes: broadcast::Sender<i64>,
    pub video_events: broadcast::Sender<()>,
    /// Wakes live connections when the server shuts down, so they close
    /// instead of holding the shutdown open.
    pub closing: tokio::sync::Notify,
    /// IDs of devices as they are revoked, so their live connections close.
    pub revoked: broadcast::Sender<String>,
    pub data_dir: PathBuf,
    pub started_at: i64,
    /// A FreshRSS refresh is running.
    pub refreshing: std::sync::atomic::AtomicBool,
    /// Seconds between scheduled refreshes; 0 until the poller starts.
    pub refresh_interval: std::sync::atomic::AtomicU64,
    pub admin: Option<admin::Admin>,
    /// Where each device is in the change feed.
    pub sync_log: progress::SyncLog,
}

impl AppState {
    pub fn open(cfg: &ServerConfig) -> Result<Arc<AppState>, String> {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| e.to_string())?;
        let secrets =
            secrets::EncryptedSecrets::open(cfg.data_dir.join("secrets.json"), &cfg.master_key)
                .map_err(|e| e.to_string())?;
        let engine = Engine::open(
            &cfg.data_dir,
            Arc::new(secrets),
            EngineOptions {
                default_cache_limit: u64::MAX,
                server_role: true,
            },
        )
        .map_err(|e| e.to_string())?;
        let store = Store::open(&cfg.data_dir).map_err(|e| e.to_string())?;
        let server_id = match store.meta_get("server_id").map_err(|e| e.to_string())? {
            Some(id) => id,
            None => {
                let id = eddy_model::ids::new_local_id();
                store
                    .meta_set("server_id", &id)
                    .map_err(|e| e.to_string())?;
                id
            }
        };
        let (changes, _) = broadcast::channel(64);
        let (video_events, _) = broadcast::channel(64);
        let (revoked, _) = broadcast::channel(16);
        Ok(Arc::new(AppState {
            engine,
            store,
            server_id,
            changes,
            video_events,
            closing: tokio::sync::Notify::new(),
            revoked,
            data_dir: cfg.data_dir.clone(),
            started_at: eddy_model::now_ms(),
            refreshing: std::sync::atomic::AtomicBool::new(false),
            refresh_interval: std::sync::atomic::AtomicU64::new(0),
            admin: cfg
                .admin_password
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(admin::Admin::new),
            sync_log: progress::SyncLog::default(),
        }))
    }

    fn info(&self) -> Result<ServerInfo, ApiError> {
        Ok(ServerInfo {
            protocol: PROTOCOL_VERSION,
            server_id: self.server_id.clone(),
            account: self.engine.server_account().map(|a| AccountInfo {
                base_url: a.base_url,
                username: a.username,
            }),
            cursor: self.engine.server_cursor().map_err(ApiError::internal)?,
            video_capable: self.engine.server_video_capable(),
            min_protocol: MIN_PROTOCOL,
            version: eddy_core::core_version(),
        })
    }

    /// A device's name for the log; its ID when it has none.
    fn device_name(&self, id: &str) -> String {
        self.store
            .devices()
            .ok()
            .and_then(|all| all.into_iter().find(|d| d.id == id))
            .map(|d| d.name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.to_string())
    }

    fn announce(&self) {
        if let Ok(cursor) = self.engine.server_cursor() {
            let _ = self.changes.send(cursor);
        }
    }

    fn announce_video_job_event(&self) {
        self.announce();
        let _ = self.video_events.send(());
    }

    /// Pulls FreshRSS now; announces new ops to listening devices. Older
    /// history then keeps loading in the background, announced page by page.
    pub async fn refresh(self: &Arc<Self>) -> Result<(), String> {
        if self.engine.server_account().is_none() {
            return Ok(());
        }
        let before = self.engine.server_cursor().unwrap_or(0);
        self.refreshing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let result = self
            .engine
            .refresh()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
        self.refreshing
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if self.engine.server_cursor().unwrap_or(0) > before {
            self.announce();
        }
        if result.is_ok() {
            self.backfill_in_background();
        }
        result
    }

    fn backfill_in_background(self: &Arc<Self>) {
        let state = self.clone();
        tokio::spawn(async move {
            let announcer = state.clone();
            let on_page: HistoryCallback = Arc::new(move |_| announcer.announce());
            // The engine logs a failed page; the next refresh resumes from it.
            let _ = state.engine.backfill_history(Some(on_page)).await;
        });
    }
}

/// Creates a one-time pairing code valid for `ttl`, stored hashed.
pub fn create_pairing_code(store: &Store, ttl: Duration) -> Result<String, String> {
    let code = secrets::new_pairing_code();
    let expires = eddy_model::now_ms() + ttl.as_millis() as i64;
    store
        .add_pairing_code(&sha256_hex(code.as_bytes()), expires)
        .map_err(|e| e.to_string())?;
    Ok(code)
}

pub struct ApiError(pub(crate) StatusCode, pub(crate) String);

impl ApiError {
    pub(crate) fn internal(e: impl std::fmt::Display) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }

    pub(crate) fn bad(e: impl std::fmt::Display) -> Self {
        ApiError(StatusCode::BAD_REQUEST, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

/// A request from a paired device (bearer token).
pub struct Device(pub String);

impl FromRequestParts<Arc<AppState>> for Device {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "需要设备令牌".into()))?;
        match state
            .store
            .device_by_token(&sha256_hex(token.trim().as_bytes()))
        {
            Ok(Some(id)) => Ok(Device(id)),
            Ok(None) => Err(ApiError(StatusCode::UNAUTHORIZED, "设备令牌无效".into())),
            Err(e) => Err(ApiError::internal(e)),
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pair", post(pair))
        .route("/v1/device", delete(unpair))
        .route("/v1/info", get(info))
        .route("/v1/account", post(set_account))
        .route("/v1/refresh", post(refresh))
        .route("/v1/subscriptions", post(add_feed))
        .route("/v1/subscriptions/{id}", delete(remove_feed))
        .route("/v1/ops", post(push).get(pull))
        .route(
            "/v1/blobs/{key}",
            get(get_blob)
                .head(head_blob)
                .put(put_blob)
                .layer(DefaultBodyLimit::max(eddy_sync::MAX_BLOB_BYTES)),
        )
        .route("/v1/config", get(config))
        .route("/v1/config/{key}", put(put_config).delete(delete_config))
        .route("/v1/search", get(search))
        .route(
            "/v1/video-jobs",
            get(list_video_jobs).post(create_video_job),
        )
        .route("/v1/video-jobs/{id}/retry", post(retry_video_job))
        .route("/v1/video-jobs/{id}/cancel", post(cancel_video_job))
        .route("/v1/video-jobs/{id}", delete(dismiss_video_job))
        .route("/v1/ws", get(ws))
        .merge(admin::routes())
        .layer(DefaultBodyLimit::max(80 * 1024 * 1024))
        .layer(axum::middleware::from_fn(require_protocol))
        // A first sync is hundreds of megabytes of JSON, which shrinks
        // several times over; the fastest level keeps the server from
        // becoming the slow part on a quick network.
        .layer(
            tower_http::compression::CompressionLayer::new()
                .quality(tower_http::CompressionLevel::Fastest),
        )
        .with_state(state)
}

async fn health(State(s): State<Arc<AppState>>) -> Result<&'static str, ApiError> {
    if s.engine.server_healthy() {
        Ok("ok")
    } else {
        Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "database unavailable".into(),
        ))
    }
}

/// Turns away clients that speak a protocol this server no longer reads,
/// before they push or pull anything. Clients older than the header do not
/// send it and count as protocol 1.
async fn require_protocol(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // Health checks and the admin page come from curl and browsers, not
    // from sync clients.
    let path = request.uri().path();
    if path == "/v1/health" || path == "/admin" || path.starts_with("/admin/") {
        return next.run(request).await;
    }
    let protocol = request
        .headers()
        .get(PROTOCOL_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(1);
    if protocol < MIN_PROTOCOL {
        return ApiError(
            StatusCode::UPGRADE_REQUIRED,
            "这个版本的 Eddy 太旧，服务端已不再支持，请先更新 Eddy 再同步。".into(),
        )
        .into_response();
    }
    next.run(request).await
}

/// Refreshes FreshRSS every `interval` in the background.
pub fn spawn_poller(state: Arc<AppState>, interval: Duration) {
    state
        .refresh_interval
        .store(interval.as_secs(), std::sync::atomic::Ordering::SeqCst);
    tokio::spawn(async move {
        // The first tick fires at once: a restarted server catches up
        // instead of waiting a full interval.
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            if let Err(e) = state.refresh().await {
                tracing::warn!("scheduled refresh failed: {e}");
            }
        }
    });
}

async fn pair(
    State(s): State<Arc<AppState>>,
    Json(req): Json<PairRequest>,
) -> Result<Json<PairResponse>, ApiError> {
    let code = req.code.trim().to_uppercase();
    if !s
        .store
        .consume_pairing_code(&sha256_hex(code.as_bytes()))
        .map_err(ApiError::internal)?
    {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "配对码无效或已过期".into(),
        ));
    }
    let token = secrets::new_token();
    let device_id = if req.device_id.trim().is_empty() {
        eddy_model::ids::new_local_id()
    } else {
        req.device_id.clone()
    };
    let name: String = req.device_name.chars().take(80).collect();
    // A device that pairs again replaces its old token.
    let _ = s.store.remove_device(&device_id);
    s.store
        .add_device(&device_id, &name, &sha256_hex(token.as_bytes()))
        .map_err(ApiError::internal)?;
    tracing::info!("paired device {name}");
    Ok(Json(PairResponse {
        token,
        info: s.info()?,
    }))
}

async fn info(State(s): State<Arc<AppState>>, _d: Device) -> Result<Json<ServerInfo>, ApiError> {
    Ok(Json(s.info()?))
}

async fn set_account(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Json(req): Json<AccountRequest>,
) -> Result<Json<ServerInfo>, ApiError> {
    s.engine
        .configure_freshrss(&req.base_url, &req.username, &req.password)
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(s.info()?))
}

/// Subscribes to a feed in the server's FreshRSS account, for a device:
/// only the server writes to FreshRSS. The new source goes out to every
/// device as an op; its articles follow from a refresh started here.
async fn add_feed(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Json(req): Json<eddy_sync::AddFeedRequest>,
) -> Result<Json<eddy_sync::AddFeedResponse>, ApiError> {
    use eddy_core::EngineError;
    let added = s
        .engine
        .add_feed(&req.url, req.folder.as_deref())
        .await
        .map_err(|e| match e {
            // The request itself: no feed there, subscribed already, no
            // account to subscribe with.
            EngineError::Invalid(_) | EngineError::NotConfigured => {
                ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
            }
            // FreshRSS out of reach or failing. Not 502: devices read that
            // as this server being offline.
            other => ApiError::internal(other),
        })?;
    s.announce();
    let state = s.clone();
    tokio::spawn(async move {
        if let Err(e) = state.refresh().await {
            tracing::warn!("refresh after adding a feed failed: {e}");
        }
    });
    Ok(Json(eddy_sync::AddFeedResponse {
        source_id: added.source_id,
        title: added.title,
        folder: added.folder,
    }))
}

/// Unsubscribes from a feed in the server's FreshRSS account, for a
/// device. The source's removal goes out to every device as an op, and its
/// articles leave their lists with it.
async fn remove_feed(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    use eddy_core::EngineError;
    s.engine.remove_feed(&id).await.map_err(|e| match e {
        // Not 404: that is what a server without this route answers.
        EngineError::NotFound | EngineError::Invalid(_) | EngineError::NotConfigured => {
            ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
        }
        other => ApiError::internal(other),
    })?;
    s.announce();
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh(State(s): State<Arc<AppState>>, _d: Device) -> Result<StatusCode, ApiError> {
    s.refresh()
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn push(
    State(s): State<Arc<AppState>>,
    Device(device): Device,
    Json(req): Json<RawPushRequest>,
) -> Result<Json<PushResponse>, ApiError> {
    let sent = req.ops.len();
    let state = s.clone();
    let from = device.clone();
    let outcome = blocking(move || state.engine.server_apply_ops(&from, &req.ops)).await?;
    if outcome.applied > 0 {
        s.announce();
    }
    if sent > 0 {
        let name = s.device_name(&device);
        match outcome.rejected.len() {
            0 => tracing::info!("device {name} uploaded {sent} changes"),
            refused => tracing::warn!("device {name} uploaded {sent} changes ({refused} refused)"),
        }
    }
    Ok(Json(PushResponse {
        applied: outcome.applied,
        cursor: outcome.cursor,
        rejected: outcome
            .rejected
            .into_iter()
            .map(|(op_id, reason)| RejectedOp { op_id, reason })
            .collect(),
    }))
}

/// A device signing out or disconnecting: its token stops working and it
/// leaves the device list.
async fn unpair(
    State(s): State<Arc<AppState>>,
    Device(device): Device,
) -> Result<StatusCode, ApiError> {
    let name = s.device_name(&device);
    s.revoke_device(&device).map_err(ApiError::internal)?;
    tracing::info!("device {name} signed out");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PullQuery {
    #[serde(default)]
    since: i64,
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    1000
}

async fn pull(
    State(s): State<Arc<AppState>>,
    Device(device): Device,
    Query(q): Query<PullQuery>,
) -> Result<Json<PullResponse>, ApiError> {
    let state = s.clone();
    let page = blocking(move || state.engine.server_ops_since(q.since, q.limit)).await?;
    s.sync_log.pulled(progress::Pulled {
        device_id: &device,
        device_name: &s.device_name(&device),
        since: q.since,
        sent: page.ops.len(),
        cursor: page.cursor,
        more: page.more,
        head: page.head,
    });
    Ok(Json(PullResponse {
        ops: page.ops,
        cursor: page.cursor,
        more: page.more,
        head: Some(page.head),
    }))
}

/// Runs database and file work off the async runtime, so a large page or
/// blob for one device does not hold up the others.
async fn blocking<T, E>(work: impl FnOnce() -> Result<T, E> + Send + 'static) -> Result<T, ApiError>
where
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

async fn get_blob(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    let state = s.clone();
    let blob = tokio::task::spawn_blocking(move || state.engine.server_blob(&key))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::bad)?;
    match blob {
        Some((bytes, mime)) => Ok(([(header::CONTENT_TYPE, mime)], bytes).into_response()),
        None => Err(ApiError(StatusCode::NOT_FOUND, "没有这个内容".into())),
    }
}

async fn head_blob(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    Ok(if s.engine.server_has_blob(&key).map_err(ApiError::bad)? {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn put_blob(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let state = s.clone();
    tokio::task::spawn_blocking(move || state.engine.server_put_blob(&key, &body, &mime))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::bad)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn config(
    State(s): State<Arc<AppState>>,
    _d: Device,
) -> Result<Json<Vec<ConfigEntry>>, ApiError> {
    let entries = s.store.config_entries().map_err(ApiError::internal)?;
    Ok(Json(
        entries
            .into_iter()
            .map(|(key, value, updated_at)| ConfigEntry {
                key,
                value,
                updated_at,
            })
            .collect(),
    ))
}

fn valid_config_key(key: &str) -> bool {
    key.len() <= 160
        && key.split_once(':').is_some_and(|(ns, name)| {
            ns == "rulepack"
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

async fn put_config(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    if !valid_config_key(&key) {
        return Err(ApiError::bad("不支持的配置项"));
    }
    s.store
        .config_set(&key, &body)
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_config(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.store.config_delete(&key).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: u32,
}

fn default_search_limit() -> u32 {
    50
}

async fn search(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<SearchHit>>, ApiError> {
    let hits = s
        .engine
        .search(&q.q, q.limit.min(200))
        .map_err(ApiError::internal)?;
    Ok(Json(
        hits.into_iter()
            .map(|h| SearchHit {
                item_id: h.item.item.id,
                snippet: h.snippet,
                highlights: h.highlights,
            })
            .collect(),
    ))
}

async fn ws(
    State(s): State<Arc<AppState>>,
    Device(device): Device,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| live(socket, s, device))
}

async fn list_video_jobs(
    State(s): State<Arc<AppState>>,
    _d: Device,
) -> Result<Json<Vec<RemoteVideoJob>>, ApiError> {
    Ok(Json(s.video_jobs()?))
}

async fn create_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Json(req): Json<CreateVideoJobRequest>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    Ok(Json(s.create_video_job(req)?))
}

async fn retry_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    Ok(Json(s.retry_video_job(&id)?))
}

async fn cancel_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    Ok(Json(s.cancel_video_job(&id)?))
}

async fn dismiss_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.dismiss_video_job(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Video jobs, as devices and the admin page both drive them.
impl AppState {
    pub(crate) fn video_jobs(&self) -> Result<Vec<RemoteVideoJob>, ApiError> {
        let jobs = self
            .engine
            .local_video_job_records(50)
            .map_err(ApiError::internal)?;
        Ok(jobs.into_iter().map(record_to_remote).collect())
    }

    fn video_job(&self, id: &str) -> Result<RemoteVideoJob, ApiError> {
        self.engine
            .local_video_job_record(id)
            .map_err(ApiError::internal)?
            .map(record_to_remote)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Job not found".into()))
    }

    fn video_notifier(self: &Arc<Self>) -> Arc<dyn Fn() + Send + Sync> {
        let state = self.clone();
        Arc::new(move || state.announce_video_job_event())
    }

    pub(crate) fn create_video_job(
        self: &Arc<Self>,
        req: CreateVideoJobRequest,
    ) -> Result<RemoteVideoJob, ApiError> {
        let src = req.source.trim();
        if !(src.starts_with("https://") || src.starts_with("http://")) {
            return Err(ApiError::bad(
                "Server video jobs only accept http:// or https:// URLs.",
            ));
        }
        let pipeline = match req.pipeline.as_deref() {
            Some("gemini_two_stage") => Some(VideoPipelineMode::GeminiTwoStage),
            Some("local_frames_two_stage") => Some(VideoPipelineMode::LocalFramesTwoStage),
            Some("auto") => Some(VideoPipelineMode::Auto),
            _ => None,
        };
        let policy = match req.transcript_policy.as_deref() {
            Some("prefer_manual_subtitles") => Some(TranscriptPolicy::PreferManualSubtitles),
            Some("always_asr") => Some(TranscriptPolicy::AlwaysAsr),
            Some("prefer_any_subtitles") => Some(TranscriptPolicy::PreferAnySubtitles),
            _ => None,
        };
        let created = self
            .engine
            .enqueue_local_video_job(
                StartVideoJobOptions {
                    source: src.to_string(),
                    target: VideoJobTarget::Local,
                    pipeline,
                    transcript_policy: policy,
                    target_language: req.target_language,
                    cookies_from_browser: None,
                },
                Some(self.video_notifier()),
            )
            .map_err(ApiError::bad)?;
        self.announce_video_job_event();
        self.video_job(&created.id)
    }

    pub(crate) fn retry_video_job(self: &Arc<Self>, id: &str) -> Result<RemoteVideoJob, ApiError> {
        self.engine
            .retry_local_video_job(id, Some(self.video_notifier()))
            .map_err(ApiError::bad)?;
        self.announce_video_job_event();
        self.video_job(id)
    }

    pub(crate) fn cancel_video_job(&self, id: &str) -> Result<RemoteVideoJob, ApiError> {
        self.engine
            .cancel_local_video_job(id)
            .map_err(ApiError::bad)?;
        self.announce_video_job_event();
        self.video_job(id)
    }

    pub(crate) fn dismiss_video_job(&self, id: &str) -> Result<(), ApiError> {
        self.engine
            .dismiss_local_video_job(id)
            .map_err(ApiError::internal)?;
        self.announce_video_job_event();
        Ok(())
    }

    /// Revokes a device: its token stops working and its live connection
    /// closes.
    pub fn revoke_device(&self, id: &str) -> Result<(), String> {
        self.store.remove_device(id).map_err(|e| e.to_string())?;
        self.sync_log.forget(id);
        let _ = self.revoked.send(id.to_string());
        Ok(())
    }
}

async fn live(mut socket: WebSocket, s: Arc<AppState>, device: String) {
    let mut gone = s.revoked.subscribe();
    let mut rx = s.changes.subscribe();
    let mut vrx = s.video_events.subscribe();
    if let Ok(cursor) = s.engine.server_cursor()
        && socket
            .send(Message::Text(
                serde_json::json!({ "cursor": cursor }).to_string().into(),
            ))
            .await
            .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            change = rx.recv() => match change {
                Ok(cursor) => {
                    if socket.send(Message::Text(serde_json::json!({ "cursor": cursor }).to_string().into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            v_ev = vrx.recv() => match v_ev {
                Ok(()) => {
                    if socket.send(Message::Text(serde_json::json!({ "video_jobs": true }).to_string().into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            _ = s.closing.notified() => break,
            // The token was only checked when the connection opened.
            revoked = gone.recv() => match revoked {
                Ok(id) if id == device => break,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}
