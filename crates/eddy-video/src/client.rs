use std::sync::LazyLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::VideoError;
use crate::redact::redact_text;
use crate::settings::{ResolvedProtocol, StageEndpoint};

static THINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());
static JSON_FENCE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)```(?:json)?\s*(\{.*\}|\[.*\])\s*```").unwrap());
static MISSING_TYPE_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\{\s*"(paragraph|subheading|bullet_list|numbered_list|quote|code|math|table|figure|figure_timestamp)"\s*,"#).unwrap()
});
static TRAILING_COMMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",(\s*[}\]])").unwrap());

#[derive(Clone, Debug)]
pub enum UserPart {
    Text(String),
    Image { mime: String, bytes: Vec<u8> },
    InlineMedia { mime: String, bytes: Vec<u8> },
    FileUri { mime: String, uri: String },
}

#[derive(Clone, Debug, Default)]
pub struct ModelOutput {
    pub text: String,
    pub elapsed_ms: u64,
}

pub fn build_http_client(timeout_secs: u64) -> Result<reqwest::Client, VideoError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| VideoError::Network(e.to_string()))
}

pub fn gemini_headers(endpoint: &StageEndpoint) -> Result<HeaderMap, VideoError> {
    let key = endpoint.api_key.trim();
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-goog-api-key",
        HeaderValue::from_str(key)
            .map_err(|_| VideoError::Config("API key contains invalid header characters.".into()))?,
    );
    if !endpoint.is_official_google_host() {
        let bearer = format!("Bearer {key}");
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer).map_err(|_| {
                VideoError::Config("API key contains invalid header characters.".into())
            })?,
        );
    }
    Ok(headers)
}

pub async fn call_stage_model(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    system_prompt: &str,
    parts: &[UserPart],
    temperature: f32,
    max_tokens: u32,
    expect_json: bool,
) -> Result<ModelOutput, VideoError> {
    if !endpoint.is_configured() {
        return Err(VideoError::Config(format!(
            "Stage model `{}` is missing URL, model ID, or API key.",
            endpoint.model
        )));
    }
    match endpoint.resolved_protocol() {
        ResolvedProtocol::OpenAi => {
            call_openai_chat(http, endpoint, system_prompt, parts, temperature, max_tokens).await
        }
        ResolvedProtocol::Gemini => {
            call_gemini_generate_content(
                http,
                endpoint,
                system_prompt,
                parts,
                temperature,
                max_tokens,
                expect_json,
            )
            .await
        }
    }
}

async fn call_openai_chat(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    system_prompt: &str,
    parts: &[UserPart],
    temperature: f32,
    max_tokens: u32,
) -> Result<ModelOutput, VideoError> {
    let base = endpoint.normalized_openai_base_url()?;
    let url = format!("{base}/chat/completions");
    let key = endpoint.api_key.trim();
    let secrets = [key];

    let user_content = if let [UserPart::Text(only_text)] = parts {
        json!(only_text)
    } else {
        let mut items = Vec::new();
        for part in parts {
            match part {
                UserPart::Text(t) => {
                    if !t.is_empty() {
                        items.push(json!({ "type": "text", "text": t }));
                    }
                }
                UserPart::Image { mime, bytes } | UserPart::InlineMedia { mime, bytes } => {
                    let data_uri = format!("data:{mime};base64,{}", BASE64.encode(bytes));
                    items.push(json!({
                        "type": "image_url",
                        "image_url": { "url": data_uri }
                    }));
                }
                UserPart::FileUri { uri, .. } => {
                    items.push(json!({
                        "type": "image_url",
                        "image_url": { "url": uri }
                    }));
                }
            }
        }
        json!(items)
    };

    let mut messages = Vec::new();
    if !system_prompt.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": system_prompt }));
    }
    messages.push(json!({ "role": "user", "content": user_content }));

    let body = json!({
        "model": endpoint.model.trim(),
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
    });

    let t0 = Instant::now();
    let resp = http
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let content_type = resp_headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let raw = resp
        .text()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    if !status.is_success() {
        return Err(VideoError::Provider(eddy_ai::model_failure(
            status.as_u16(),
            &content_type,
            &raw,
            &secrets,
            Some(&resp_headers),
        )));
    }

    let parsed: Value = serde_json::from_str(&raw).map_err(|_| {
        VideoError::Format(format!(
            "Model endpoint returned non-JSON body: {}",
            redact_text(&raw, &secrets)
        ))
    })?;

    let text = extract_openai_message_text(&parsed)?;

    Ok(ModelOutput {
        text,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn extract_openai_message_text(parsed: &Value) -> Result<String, VideoError> {
    let msg = &parsed["choices"][0]["message"]["content"];
    if let Some(s) = msg.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(arr) = msg.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.trim().is_empty() {
            return Ok(joined.trim().to_string());
        }
    }
    Err(VideoError::Format("Model returned empty message content.".into()))
}

pub async fn call_gemini_generate_content(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    system_prompt: &str,
    parts: &[UserPart],
    temperature: f32,
    max_tokens: u32,
    expect_json: bool,
) -> Result<ModelOutput, VideoError> {
    let url = endpoint.gemini_generate_content_url()?;
    let headers = gemini_headers(endpoint)?;
    let key = endpoint.api_key.trim();
    let secrets = [key];

    let mut user_parts = Vec::new();
    for part in parts {
        match part {
            UserPart::FileUri { mime, uri } => {
                if mime.is_empty() {
                    user_parts.push(json!({ "fileData": { "fileUri": uri } }));
                } else {
                    user_parts.push(json!({
                        "fileData": { "mimeType": mime, "fileUri": uri }
                    }));
                }
            }
            UserPart::Image { mime, bytes } | UserPart::InlineMedia { mime, bytes } => {
                user_parts.push(json!({
                    "inlineData": {
                        "mimeType": mime,
                        "data": BASE64.encode(bytes),
                    }
                }));
            }
            UserPart::Text(t) => {
                if !t.trim().is_empty() {
                    user_parts.push(json!({ "text": t }));
                }
            }
        }
    }

    let mut gen_config = json!({
        "temperature": temperature,
        "maxOutputTokens": max_tokens,
    });
    if expect_json {
        gen_config["responseMimeType"] = json!("application/json");
    }

    let mut payload = json!({
        "contents": [{ "role": "user", "parts": user_parts }],
        "generationConfig": gen_config,
    });
    if !system_prompt.trim().is_empty() {
        payload["systemInstruction"] = json!({
            "parts": [{ "text": system_prompt }]
        });
    }

    let t0 = Instant::now();
    let mut resp = http
        .post(&url)
        .headers(headers.clone())
        .json(&payload)
        .send()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    let mut status = resp.status();
    let mut resp_headers = resp.headers().clone();
    let mut raw = resp
        .text()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    if status.as_u16() == 400 && expect_json && raw.contains("responseMimeType") {
        if let Some(obj) = payload["generationConfig"].as_object_mut() {
            obj.remove("responseMimeType");
        }
        resp = http
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;
        status = resp.status();
        resp_headers = resp.headers().clone();
        raw = resp
            .text()
            .await
            .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;
    }

    let content_type = resp_headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !status.is_success() {
        return Err(VideoError::Provider(eddy_ai::model_failure(
            status.as_u16(),
            &content_type,
            &raw,
            &secrets,
            Some(&resp_headers),
        )));
    }

    let parsed: Value = serde_json::from_str(&raw).map_err(|_| {
        VideoError::Format(format!(
            "Gemini endpoint returned non-JSON body: {}",
            redact_text(&raw, &secrets)
        ))
    })?;

    let candidates = parsed["candidates"]
        .as_array()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            VideoError::Provider(format!(
                "Gemini endpoint returned no candidates: {}",
                redact_text(&raw, &secrets)
            ))
        })?;

    let parts_arr = candidates[0]["content"]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut pieces: Vec<&str> = parts_arr
        .iter()
        .filter(|p| p["thought"].as_bool() != Some(true))
        .filter_map(|p| p["text"].as_str())
        .filter(|t| !t.is_empty())
        .collect();
    if pieces.is_empty() {
        pieces = parts_arr
            .iter()
            .filter_map(|p| p["text"].as_str())
            .filter(|t| !t.is_empty())
            .collect();
    }
    let text = pieces.join("\n").trim().to_string();
    if text.is_empty() {
        return Err(VideoError::Format("Gemini response contained no text output.".into()));
    }

    Ok(ModelOutput {
        text,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

/// Tests connectivity and credentials for a stage endpoint.
pub async fn test_stage_endpoint(endpoint: &StageEndpoint) -> Result<u64, VideoError> {
    let http = build_http_client(45)?;
    let model_lower = endpoint.model.to_ascii_lowercase();
    if endpoint.resolved_protocol() == ResolvedProtocol::OpenAi
        && (model_lower.contains("asr")
            || model_lower.contains("whisper")
            || model_lower.contains("transcribe"))
    {
        // Dedicated ASR models do not accept text chat; verify auth/endpoint via `/models` or short probe
        let base = endpoint.normalized_openai_base_url()?;
        let secrets = [endpoint.api_key.trim()];
        let t0 = Instant::now();
        let resp = http
            .get(format!("{base}/models"))
            .bearer_auth(endpoint.api_key.trim())
            .send()
            .await
            .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;
        if resp.status().is_success() {
            return Ok(t0.elapsed().as_millis() as u64);
        }
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let ct = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let raw = resp.text().await.unwrap_or_default();
        return Err(VideoError::Provider(eddy_ai::model_failure(
            status,
            &ct,
            &raw,
            &secrets,
            Some(&headers),
        )));
    }

    let out = call_stage_model(
        &http,
        endpoint,
        "Reply strictly with JSON.",
        &[UserPart::Text(r#"Return {"ok": true}"#.into())],
        0.0,
        512,
        true,
    )
    .await?;
    Ok(out.elapsed_ms)
}

fn repair_common_llm_json(snippet: &str) -> String {
    let fixed = MISSING_TYPE_KEY_RE.replace_all(snippet, r#"{"type": "$1","#);
    TRAILING_COMMA_RE.replace_all(&fixed, "$1").into_owned()
}

/// Extracts and parses a JSON object from model output, repairing minor syntax slips.
pub fn extract_json_object(raw_text: &str) -> Result<Value, VideoError> {
    let stripped = THINK_RE.replace_all(raw_text.trim(), "");
    let mut cleaned = stripped.trim().to_string();
    if let Some(cap) = JSON_FENCE_RE.captures(&cleaned) {
        cleaned = cap[1].trim().to_string();
    }

    for candidate in [cleaned.clone(), repair_common_llm_json(&cleaned)] {
        if let Ok(val) = serde_json::from_str::<Value>(&candidate)
            && val.is_object()
        {
            return Ok(val);
        }
    }

    if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}'))
        && end > start
    {
        let snippet = &cleaned[start..=end];
        let mut last_err = String::new();
        for candidate in [snippet.to_string(), repair_common_llm_json(snippet)] {
            match serde_json::from_str::<Value>(&candidate) {
                Ok(val) if val.is_object() => return Ok(val),
                Ok(_) => last_err = "Top-level JSON value is not an object".into(),
                Err(e) => last_err = e.to_string(),
            }
        }
        let preview: String = snippet.chars().take(240).collect();
        return Err(VideoError::Format(format!(
            "Model returned invalid JSON ({last_err}): {preview}"
        )));
    }

    let preview: String = cleaned.chars().take(240).collect();
    Err(VideoError::Format(format!(
        "No JSON object found in model output: {preview}"
    )))
}
