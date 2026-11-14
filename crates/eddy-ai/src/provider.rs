use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{AiError, redact::model_failure};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    #[default]
    Auto,
    OpenAi,
    Gemini,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedAiProtocol {
    OpenAi,
    Gemini,
}

pub fn resolve_protocol(protocol: AiProtocol, url: &str, _model: &str) -> ResolvedAiProtocol {
    match protocol {
        AiProtocol::OpenAi => return ResolvedAiProtocol::OpenAi,
        AiProtocol::Gemini => return ResolvedAiProtocol::Gemini,
        AiProtocol::Auto => {}
    }
    let raw = url.trim();
    if let Ok(parsed) = url::Url::parse(raw) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = parsed.path().trim_end_matches('/').to_ascii_lowercase();
        if path.ends_with("/openai")
            || path.contains("/v1beta/openai")
            || path.ends_with("/chat/completions")
            || path.contains("/compatible-mode")
            || path.ends_with("/v1")
        {
            return ResolvedAiProtocol::OpenAi;
        }
        if host.ends_with("googleapis.com")
            || path.ends_with("/v1beta")
            || path.ends_with("/v1alpha")
            || path.contains("/v1beta/")
            || path.contains(":generatecontent")
        {
            return ResolvedAiProtocol::Gemini;
        }
    }
    ResolvedAiProtocol::OpenAi
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        ChatMessage {
            role: "system",
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage {
            role: "user",
            content: content.into(),
        }
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

fn validate_url(input: &str) -> Result<url::Url, AiError> {
    let url = url::Url::parse(input.trim()).map_err(|_| {
        AiError::Config("请输入完整的模型 API 地址，例如 https://api.example.com/v1。".into())
    })?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(AiError::Config(
            "模型 API 地址需要以 https:// 或 http:// 开头。".into(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AiError::Config(
            "模型 API 地址里不能包含账号、参数或锚点。".into(),
        ));
    }
    Ok(url)
}

/// Normalizes a Chat Completions base URL: a bare host gets `/v1`, a full
/// `/chat/completions` endpoint is trimmed, custom prefixes stay intact.
/// Plain HTTP is allowed for local model servers.
pub fn normalize_base(input: &str) -> Result<String, AiError> {
    let mut url = validate_url(input)?;
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

/// Normalizes a Gemini native `/v1beta` base URL: bare hosts get `/v1beta`,
/// and `/models/...` suffixes are stripped back to the version root.
pub fn normalize_gemini_base(input: &str) -> Result<String, AiError> {
    let mut url = validate_url(input)?;
    let mut path = url.path().trim_end_matches('/').to_string();
    if let Some((before, _)) = path.split_once("/models/") {
        path = before.trim_end_matches('/').to_string();
    } else if let Some(stripped) = path.strip_suffix("/models") {
        path = stripped.trim_end_matches('/').to_string();
    }
    if !(path.ends_with("/v1beta") || path.ends_with("/v1alpha") || path.ends_with("/v1")) {
        path = if path.is_empty() {
            "/v1beta".into()
        } else {
            format!("{path}/v1beta")
        };
    }
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub fn normalize_provider_url(
    protocol: AiProtocol,
    input: &str,
    model: &str,
) -> Result<String, AiError> {
    match resolve_protocol(protocol, input, model) {
        ResolvedAiProtocol::OpenAi => normalize_base(input),
        ResolvedAiProtocol::Gemini => normalize_gemini_base(input),
    }
}

#[derive(Clone)]
pub struct ChatProvider {
    http: reqwest::Client,
    protocol: ResolvedAiProtocol,
    base_url: String,
    model: String,
    api_key: String,
}

impl std::fmt::Debug for ChatProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatProvider")
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"[已隐藏]")
            .finish()
    }
}

impl ChatProvider {
    pub fn new(
        protocol: AiProtocol,
        base_url: &str,
        model: &str,
        api_key: &str,
    ) -> Result<Self, AiError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(AiError::Config("请填写模型 ID。".into()));
        }
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(AiError::Config("请填写 API Key。".into()));
        }
        let resolved = resolve_protocol(protocol, base_url, model);
        let normalized_base = match resolved {
            ResolvedAiProtocol::OpenAi => normalize_base(base_url)?,
            ResolvedAiProtocol::Gemini => normalize_gemini_base(base_url)?,
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AiError::Network(e.to_string()))?;
        Ok(ChatProvider {
            http,
            protocol: resolved,
            base_url: normalized_base,
            model: model.to_string(),
            api_key: api_key.to_string(),
        })
    }

    fn is_official_google_host(&self) -> bool {
        url::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
            .is_some_and(|h| h.ends_with("googleapis.com"))
    }

    fn gemini_headers(&self) -> Result<HeaderMap, AiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| AiError::Config("API Key 包含无效的请求头字符。".into()))?,
        );
        if !self.is_official_google_host() {
            let bearer = format!("Bearer {}", self.api_key);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&bearer)
                    .map_err(|_| AiError::Config("API Key 包含无效的请求头字符。".into()))?,
            );
        }
        Ok(headers)
    }

    async fn complete_openai(&self, messages: &[ChatMessage]) -> Result<String, AiError> {
        let body =
            serde_json::json!({ "model": self.model, "temperature": 0.2, "messages": messages });
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AiError::Network("模型请求超时，请稍后重试。".into())
                } else {
                    AiError::Network(crate::redact::safe_detail(&e.to_string(), &[&self.api_key]))
                }
            })?;
        completion_text(response, &[&self.api_key]).await
    }

    async fn complete_gemini(&self, messages: &[ChatMessage]) -> Result<String, AiError> {
        let clean_model = self.model.trim_start_matches("models/");
        if clean_model.is_empty() {
            return Err(AiError::Config("请填写模型 ID。".into()));
        }
        let url = format!("{}/models/{clean_model}:generateContent", self.base_url);
        let headers = self.gemini_headers()?;

        let system_parts: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role == "system" && !m.content.trim().is_empty())
            .map(|m| serde_json::json!({ "text": m.content }))
            .collect();
        let user_parts: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| m.role != "system" && !m.content.trim().is_empty())
            .map(|m| serde_json::json!({ "text": m.content }))
            .collect();

        let mut body = serde_json::json!({
            "contents": [{ "role": "user", "parts": user_parts }],
            "generationConfig": { "temperature": 0.2 },
        });
        if !system_parts.is_empty() {
            body["systemInstruction"] = serde_json::json!({ "parts": system_parts });
        }

        let response = self
            .http
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AiError::Network("模型请求超时，请稍后重试。".into())
                } else {
                    AiError::Network(crate::redact::safe_detail(&e.to_string(), &[&self.api_key]))
                }
            })?;

        let status = response.status().as_u16();
        let resp_headers = response.headers().clone();
        let content_type = resp_headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let raw = response
            .text()
            .await
            .map_err(|e| AiError::Network(crate::redact::safe_detail(&e.to_string(), &[&self.api_key])))?;
        parse_gemini_completion(status, &content_type, &raw, &[&self.api_key], Some(&resp_headers))
    }
}

#[async_trait]
impl Provider for ChatProvider {
    fn cache_identity(&self) -> String {
        let proto = match self.protocol {
            ResolvedAiProtocol::OpenAi => "openai",
            ResolvedAiProtocol::Gemini => "gemini",
        };
        format!("{proto}\u{1f}{}\u{1f}{}", self.base_url, self.model)
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[ChatMessage]) -> Result<String, AiError> {
        match self.protocol {
            ResolvedAiProtocol::OpenAi => self.complete_openai(messages).await,
            ResolvedAiProtocol::Gemini => self.complete_gemini(messages).await,
        }
    }
}

#[derive(Clone)]
pub struct OpenAiCompatible(ChatProvider);

impl std::fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatible")
            .field("base_url", &self.0.base_url)
            .field("model", &self.0.model)
            .field("api_key", &"[已隐藏]")
            .finish()
    }
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Result<Self, AiError> {
        Ok(OpenAiCompatible(ChatProvider::new(
            AiProtocol::OpenAi,
            base_url,
            model,
            api_key,
        )?))
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn cache_identity(&self) -> String {
        self.0.cache_identity()
    }

    fn model(&self) -> &str {
        self.0.model()
    }

    async fn complete(&self, messages: &[ChatMessage]) -> Result<String, AiError> {
        self.0.complete(messages).await
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
    content: Option<serde_json::Value>,
}

fn extract_message_content(content: serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Array(items) => {
            let joined: String = items
                .iter()
                .filter_map(|p| p.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

/// Extracts the assistant text from a Chat Completions response, turning
/// every failure mode (HTTP errors, gateway HTML pages, malformed JSON) into
/// a readable, redacted error.
pub async fn completion_text(
    response: reqwest::Response,
    secrets: &[&str],
) -> Result<String, AiError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let raw = response
        .text()
        .await
        .map_err(|e| AiError::Network(crate::redact::safe_detail(&e.to_string(), secrets)))?;
    parse_completion(status, &content_type, &raw, secrets, Some(&headers))
}

fn check_response_envelope(
    status: u16,
    content_type: &str,
    raw: &str,
    secrets: &[&str],
    headers: Option<&reqwest::header::HeaderMap>,
) -> Result<serde_json::Value, AiError> {
    let html = content_type.contains("text/html") || {
        let t = raw.trim_start().to_lowercase();
        t.starts_with("<!doctype") || t.starts_with("<html")
    };
    if !(200..300).contains(&status) || html {
        return Err(AiError::Provider(model_failure(
            status,
            content_type,
            raw,
            secrets,
            headers,
        )));
    }
    if raw.len() > 1_000_000 {
        return Err(AiError::Provider(
            "模型响应过长，请减少单次处理的内容。".into(),
        ));
    }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| {
        AiError::Provider("模型服务未返回有效 JSON，请检查 API Base URL 和接口兼容性。".into())
    })?;
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Err(AiError::Provider(model_failure(
            status,
            content_type,
            raw,
            secrets,
            headers,
        )));
    }
    Ok(value)
}

pub fn parse_completion(
    status: u16,
    content_type: &str,
    raw: &str,
    secrets: &[&str],
    headers: Option<&reqwest::header::HeaderMap>,
) -> Result<String, AiError> {
    let value = check_response_envelope(status, content_type, raw, secrets, headers)?;
    let completion: Completion = serde_json::from_value(value)
        .map_err(|_| AiError::Provider("模型没有返回可用结果，请更换模型或重试。".into()))?;
    completion
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .and_then(extract_message_content)
        .filter(|t| t.len() <= 250_000)
        .ok_or_else(|| AiError::Provider("模型没有返回可用结果，请更换模型或重试。".into()))
}

pub fn parse_gemini_completion(
    status: u16,
    content_type: &str,
    raw: &str,
    secrets: &[&str],
    headers: Option<&reqwest::header::HeaderMap>,
) -> Result<String, AiError> {
    let value = check_response_envelope(status, content_type, raw, secrets, headers)?;
    let candidates = value
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| AiError::Provider("模型没有返回可用结果，请更换模型或重试。".into()))?;
    let parts = candidates[0]["content"]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut pieces: Vec<&str> = parts
        .iter()
        .filter(|p| p["thought"].as_bool() != Some(true))
        .filter_map(|p| p["text"].as_str())
        .filter(|t| !t.is_empty())
        .collect();
    if pieces.is_empty() {
        pieces = parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .filter(|t| !t.is_empty())
            .collect();
    }
    let text = pieces.join("\n");
    if text.trim().is_empty() || text.len() > 250_000 {
        return Err(AiError::Provider(
            "模型没有返回可用结果，请更换模型或重试。".into(),
        ));
    }
    Ok(text)
}
