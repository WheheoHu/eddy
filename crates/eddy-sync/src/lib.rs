//! Sync protocol between Eddy clients and `eddy-server`: request and
//! response types shared by both sides, and the client used by the engine.
//!
//! Everything that changes is an op. Clients push their local ops and pull
//! the server's op log after a cursor (the server's op sequence number);
//! merges are per-field last-writer-wins by HLC on both sides. Content blobs
//! travel separately, on demand.

use std::time::Duration;

use eddy_model::Op;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Largest content blob a server accepts (imported books and PDFs).
pub const MAX_BLOB_BYTES: usize = 256 * 1024 * 1024;

/// Blob transfers can be large; other requests keep the client's timeout.
const BLOB_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("{0}")]
    Config(String),
    #[error("服务端拒绝了这台设备，请重新配对。")]
    Unauthorized,
    #[error("服务端返回 HTTP {0}：{1}")]
    Http(u16, String),
    #[error("连接服务端失败：{0}")]
    Network(String),
    #[error("服务端返回的数据格式不正确：{0}")]
    Format(String),
}

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub base_url: String,
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol: u32,
    pub server_id: String,
    pub account: Option<AccountInfo>,
    pub cursor: i64,
    #[serde(default)]
    pub video_capable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateVideoJobRequest {
    pub source: String,
    #[serde(default)]
    pub pipeline: Option<String>,
    #[serde(default)]
    pub transcript_policy: Option<String>,
    #[serde(default)]
    pub target_language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteVideoJob {
    pub id: String,
    pub source: String,
    pub title: String,
    pub pipeline: String,
    pub transcript_policy: String,
    pub target_language: String,
    pub phase: String,
    pub status: String,
    pub completed_chapters: u32,
    pub total_chapters: u32,
    pub detail: String,
    pub error: Option<String>,
    pub result_series_id: Option<String>,
    pub result_item_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairRequest {
    pub code: String,
    pub device_id: String,
    pub device_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairResponse {
    pub token: String,
    pub info: ServerInfo,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountRequest {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushRequest {
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushResponse {
    pub applied: u32,
    pub cursor: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullResponse {
    pub ops: Vec<Op>,
    pub cursor: i64,
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub item_id: String,
    pub snippet: String,
    pub highlights: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

/// Normalizes a server address: scheme and host required, no credentials,
/// queries or fragments; trailing slash removed.
pub fn normalize_server_url(input: &str) -> Result<String> {
    let url = url::Url::parse(input.trim()).map_err(|_| {
        SyncError::Config("请输入完整的服务端地址，例如 http://127.0.0.1:8787。".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(SyncError::Config(
            "服务端地址需要以 http:// 或 https:// 开头。".into(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SyncError::Config(
            "服务端地址里不能包含账号、参数或锚点。".into(),
        ));
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub struct SyncClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl SyncClient {
    pub fn new(base_url: &str, token: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SyncError::Network(e.to_string()))?;
        Ok(SyncClient {
            http,
            base: normalize_server_url(base_url)?,
            token,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/{path}", self.base)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = self.auth(req).send().await.map_err(|e| {
            SyncError::Network(if e.is_timeout() {
                "请求超时".into()
            } else {
                e.without_url().to_string()
            })
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SyncError::Unauthorized);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ErrorBody>(&body)
                .map(|b| b.error)
                .unwrap_or_else(|_| body.chars().take(200).collect());
            return Err(SyncError::Http(status.as_u16(), message));
        }
        Ok(response)
    }

    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T> {
        let response = self.send(req).await?;
        response
            .json()
            .await
            .map_err(|e| SyncError::Format(e.to_string()))
    }

    pub async fn pair(&self, req: &PairRequest) -> Result<PairResponse> {
        self.json(self.http.post(self.url("pair")).json(req)).await
    }

    pub async fn info(&self) -> Result<ServerInfo> {
        self.json(self.http.get(self.url("info"))).await
    }

    pub async fn set_account(&self, req: &AccountRequest) -> Result<ServerInfo> {
        self.json(self.http.post(self.url("account")).json(req))
            .await
    }

    pub async fn refresh(&self) -> Result<()> {
        self.send(self.http.post(self.url("refresh")))
            .await
            .map(|_| ())
    }

    pub async fn push(&self, ops: Vec<Op>) -> Result<PushResponse> {
        self.json(self.http.post(self.url("ops")).json(&PushRequest { ops }))
            .await
    }

    pub async fn pull(&self, since: i64, limit: u32) -> Result<PullResponse> {
        self.json(
            self.http
                .get(self.url("ops"))
                .query(&[("since", since.to_string()), ("limit", limit.to_string())]),
        )
        .await
    }

    pub async fn has_blob(&self, key: &str) -> Result<bool> {
        let response = self
            .auth(self.http.head(self.url(&format!("blobs/{key}"))))
            .send()
            .await
            .map_err(|e| SyncError::Network(e.without_url().to_string()))?;
        match response.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            401 => Err(SyncError::Unauthorized),
            s => Err(SyncError::Http(s, String::new())),
        }
    }

    pub async fn put_blob(&self, key: &str, bytes: Vec<u8>, mime: &str) -> Result<()> {
        self.send(
            self.http
                .put(self.url(&format!("blobs/{key}")))
                .timeout(BLOB_TIMEOUT)
                .header("content-type", mime)
                .body(bytes),
        )
        .await
        .map(|_| ())
    }

    pub async fn get_blob(&self, key: &str) -> Result<Option<(Vec<u8>, String)>> {
        match self
            .send(
                self.http
                    .get(self.url(&format!("blobs/{key}")))
                    .timeout(BLOB_TIMEOUT),
            )
            .await
        {
            Ok(response) => {
                let mime = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| SyncError::Network(e.to_string()))?;
                Ok(Some((bytes.to_vec(), mime)))
            }
            Err(SyncError::Http(404, _)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn config(&self) -> Result<Vec<ConfigEntry>> {
        self.json(self.http.get(self.url("config"))).await
    }

    pub async fn put_config(&self, key: &str, value: &str) -> Result<()> {
        self.send(
            self.http
                .put(self.url(&format!("config/{key}")))
                .body(value.to_string()),
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_config(&self, key: &str) -> Result<()> {
        self.send(self.http.delete(self.url(&format!("config/{key}"))))
            .await
            .map(|_| ())
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        self.json(
            self.http
                .get(self.url("search"))
                .query(&[("q", query.to_string()), ("limit", limit.to_string())]),
        )
        .await
    }

    pub async fn video_jobs(&self) -> Result<Vec<RemoteVideoJob>> {
        self.json(self.http.get(self.url("video-jobs"))).await
    }

    pub async fn create_video_job(&self, req: &CreateVideoJobRequest) -> Result<RemoteVideoJob> {
        self.json(self.http.post(self.url("video-jobs")).json(req))
            .await
    }

    pub async fn retry_video_job(&self, id: &str) -> Result<RemoteVideoJob> {
        self.json(self.http.post(self.url(&format!("video-jobs/{id}/retry"))))
            .await
    }

    pub async fn cancel_video_job(&self, id: &str) -> Result<RemoteVideoJob> {
        self.json(self.http.post(self.url(&format!("video-jobs/{id}/cancel"))))
            .await
    }

    pub async fn dismiss_video_job(&self, id: &str) -> Result<()> {
        self.send(self.http.delete(self.url(&format!("video-jobs/{id}"))))
            .await
            .map(|_| ())
    }

    /// Opens the live channel and calls `on_change` with the server cursor
    /// whenever new ops land. Returns when the connection closes.
    pub async fn listen(&self, mut on_change: impl FnMut(i64) + Send) -> Result<()> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let ws = self.url("ws").replacen("http", "ws", 1);
        let mut request = ws
            .into_client_request()
            .map_err(|e| SyncError::Config(e.to_string()))?;
        if let Some(t) = &self.token {
            request.headers_mut().insert(
                "authorization",
                format!("Bearer {t}")
                    .parse()
                    .map_err(|_| SyncError::Unauthorized)?,
            );
        }
        let (mut stream, _) =
            tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| match e {
                    tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 401 => {
                        SyncError::Unauthorized
                    }
                    e => SyncError::Network(e.to_string()),
                })?;
        while let Some(message) = stream.next().await {
            let message = message.map_err(|e| SyncError::Network(e.to_string()))?;
            if let Ok(text) = message.to_text()
                && let Ok(n) = serde_json::from_str::<serde_json::Value>(text)
            {
                if let Some(cursor) = n["cursor"].as_i64() {
                    on_change(cursor);
                } else if n["video_jobs"].as_bool() == Some(true) {
                    on_change(-1);
                }
            }
        }
        Ok(())
    }
}
