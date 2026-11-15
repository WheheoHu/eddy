//! eddy-server: optional sync hub for Eddy clients. It owns the FreshRSS
//! account, keeps the op log every device syncs with, stores content blobs,
//! distributes private configuration and searches the full history.

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
    AccountInfo, AccountRequest, ConfigEntry, CreateVideoJobRequest, ErrorBody, PROTOCOL_VERSION,
    PairRequest, PairResponse, PullResponse, PushRequest, PushResponse, RemoteVideoJob, SearchHit,
    ServerInfo,
};
use serde::Deserialize;
use tokio::sync::broadcast;

pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub master_key: [u8; 32],
}

pub struct AppState {
    pub engine: Engine,
    /// Separate handle for devices, pairing codes and config.
    pub store: Store,
    pub server_id: String,
    pub changes: broadcast::Sender<i64>,
    pub video_events: broadcast::Sender<()>,
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
        Ok(Arc::new(AppState {
            engine,
            store,
            server_id,
            changes,
            video_events,
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
        })
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
        let result = self
            .engine
            .refresh()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
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

pub struct ApiError(StatusCode, String);

impl ApiError {
    fn internal(e: impl std::fmt::Display) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }

    fn bad(e: impl std::fmt::Display) -> Self {
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
        .route("/v1/health", get(|| async { "ok" }))
        .route("/v1/pair", post(pair))
        .route("/v1/info", get(info))
        .route("/v1/account", post(set_account))
        .route("/v1/refresh", post(refresh))
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
        .layer(DefaultBodyLimit::max(80 * 1024 * 1024))
        .with_state(state)
}

/// Refreshes FreshRSS every `interval` in the background.
pub fn spawn_poller(state: Arc<AppState>, interval: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await;
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

async fn refresh(State(s): State<Arc<AppState>>, _d: Device) -> Result<StatusCode, ApiError> {
    s.refresh()
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn push(
    State(s): State<Arc<AppState>>,
    Device(device): Device,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, ApiError> {
    let (applied, cursor) = s
        .engine
        .server_apply_ops(&device, &req.ops)
        .map_err(ApiError::bad)?;
    if applied > 0 {
        s.announce();
    }
    Ok(Json(PushResponse { applied, cursor }))
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
    _d: Device,
    Query(q): Query<PullQuery>,
) -> Result<Json<PullResponse>, ApiError> {
    let (ops, cursor, more) = s
        .engine
        .server_ops_since(q.since, q.limit)
        .map_err(ApiError::internal)?;
    Ok(Json(PullResponse { ops, cursor, more }))
}

async fn get_blob(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    match s.engine.server_blob(&key).map_err(ApiError::bad)? {
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
        .unwrap_or("application/octet-stream");
    s.engine
        .server_put_blob(&key, &body, mime)
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

async fn ws(State(s): State<Arc<AppState>>, _d: Device, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| live(socket, s))
}

async fn list_video_jobs(
    State(s): State<Arc<AppState>>,
    _d: Device,
) -> Result<Json<Vec<RemoteVideoJob>>, ApiError> {
    let jobs = s
        .engine
        .local_video_job_records(50)
        .map_err(ApiError::internal)?;
    Ok(Json(jobs.into_iter().map(record_to_remote).collect()))
}

async fn create_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Json(req): Json<CreateVideoJobRequest>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
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

    let state_cb = s.clone();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || state_cb.announce_video_job_event());
    let created = s
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
            Some(notify),
        )
        .map_err(ApiError::bad)?;
    s.announce_video_job_event();
    let rec = s
        .engine
        .local_video_job_record(&created.id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("created video job missing"))?;
    Ok(Json(record_to_remote(rec)))
}

async fn retry_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    let state_cb = s.clone();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || state_cb.announce_video_job_event());
    s.engine
        .retry_local_video_job(&id, Some(notify))
        .map_err(ApiError::bad)?;
    s.announce_video_job_event();
    let rec = s
        .engine
        .local_video_job_record(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Job not found".into()))?;
    Ok(Json(record_to_remote(rec)))
}

async fn cancel_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<Json<RemoteVideoJob>, ApiError> {
    s.engine
        .cancel_local_video_job(&id)
        .map_err(ApiError::bad)?;
    s.announce_video_job_event();
    let rec = s
        .engine
        .local_video_job_record(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Job not found".into()))?;
    Ok(Json(record_to_remote(rec)))
}

async fn dismiss_video_job(
    State(s): State<Arc<AppState>>,
    _d: Device,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.engine
        .dismiss_local_video_job(&id)
        .map_err(ApiError::internal)?;
    s.announce_video_job_event();
    Ok(StatusCode::NO_CONTENT)
}

async fn live(mut socket: WebSocket, s: Arc<AppState>) {
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
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}
