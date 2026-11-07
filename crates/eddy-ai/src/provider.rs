use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{AiError, redact::model_failure};

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        ChatMessage { role: "system", content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage { role: "user", content: content.into() }
    }
}

/// A chat model service. New protocols implement this trait.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Identity used in cache keys: changing the endpoint or model yields
    /// new results.
    fn cache_identity(&self) -> String;

    fn model(&self) -> &str;

    async fn complete(&self, messages: &[ChatMessage]) -> Result<String, AiError>;
}

/// Normalizes a Chat Completions base URL: a bare host gets `/v1`, a full
/// `/chat/completions` endpoint is trimmed, custom prefixes stay intact.
/// Plain HTTP is allowed for local model servers.
pub fn normalize_base(input: &str) -> Result<String, AiError> {
    let mut url = url::Url::parse(input.trim()).map_err(|_| AiError::Config("请输入完整的模型 API 地址，例如 https://api.example.com/v1。".into()))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(AiError::Config("模型 API 地址需要以 https:// 或 http:// 开头。".into()));
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(AiError::Config("模型 API 地址里不能包含账号、参数或锚点。".into()));
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    if let Some(stripped) = path.strip_suffix("/chat/completions") {
        path = stripped.to_string();
    }
    if path.is_empty() {
        path = "/v1".into();
    }
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

#[derive(Clone)]
pub struct OpenAiCompatible {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
}

impl std::fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatible").field("base_url", &self.base_url).field("model", &self.model).field("api_key", &"[已隐藏]").finish()
    }
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Result<Self, AiError> {
        if model.trim().is_empty() {
            return Err(AiError::Config("请填写模型 ID。".into()));
        }
        if api_key.trim().is_empty() {
            return Err(AiError::Config("请填写 API Key。".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AiError::Network(e.to_string()))?;
        Ok(OpenAiCompatible { http, base_url: normalize_base(base_url)?, model: model.trim().to_string(), api_key: api_key.trim().to_string() })
    }
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn cache_identity(&self) -> String {
        format!("openai\u{1f}{}\u{1f}{}", self.base_url, self.model)
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[ChatMessage]) -> Result<String, AiError> {
        let body = serde_json::json!({ "model": self.model, "temperature": 0.2, "messages": messages });
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() { AiError::Network("模型请求超时，请稍后重试。".into()) } else { AiError::Network(crate::redact::safe_detail(&e.to_string(), &[&self.api_key])) }
            })?;
        completion_text(response, &[&self.api_key]).await
    }
}

/// Extracts the assistant text from a Chat Completions response, turning
/// every failure mode (HTTP errors, gateway HTML pages, malformed JSON) into
/// a readable, redacted error.
pub async fn completion_text(response: reqwest::Response, secrets: &[&str]) -> Result<String, AiError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let content_type = headers.get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let raw = response.text().await.map_err(|e| AiError::Network(e.to_string()))?;
    parse_completion(status, &content_type, &raw, secrets, Some(&headers))
}

pub fn parse_completion(status: u16, content_type: &str, raw: &str, secrets: &[&str], headers: Option<&reqwest::header::HeaderMap>) -> Result<String, AiError> {
    let html = content_type.contains("text/html") || {
        let t = raw.trim_start().to_lowercase();
        t.starts_with("<!doctype") || t.starts_with("<html")
    };
    if !(200..300).contains(&status) || html {
        return Err(AiError::Provider(model_failure(status, content_type, raw, secrets, headers)));
    }
    if raw.len() > 1_000_000 {
        return Err(AiError::Provider("模型响应过长，请减少单次处理的内容。".into()));
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| AiError::Provider("模型服务未返回有效 JSON，请检查 API Base URL 和接口兼容性。".into()))?;
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Err(AiError::Provider(model_failure(status, content_type, raw, secrets, headers)));
    }
    let completion: Completion = serde_json::from_value(value).map_err(|_| AiError::Provider("模型没有返回可用结果，请更换模型或重试。".into()))?;
    completion
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|t| t.len() <= 250_000)
        .ok_or_else(|| AiError::Provider("模型没有返回可用结果，请更换模型或重试。".into()))
}
