use std::error::Error as _;
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
static TRAILING_COMMA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",(\s*[}\]])").unwrap());

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

/// Builds a streaming-friendly HTTP client with a per-chunk idle `read_timeout` (reset on every
/// streamed SSE frame) plus a generous outer `total_timeout` ceiling and HTTP/2 PING keepalives.
pub fn build_streaming_http_client(
    read_timeout_secs: u64,
    total_timeout_secs: u64,
) -> Result<reqwest::Client, VideoError> {
    reqwest::Client::builder()
        .read_timeout(Duration::from_secs(read_timeout_secs))
        .timeout(Duration::from_secs(total_timeout_secs))
        .connect_timeout(Duration::from_secs(20))
        .tcp_keepalive(Duration::from_secs(30))
        .http2_keep_alive_interval(Some(Duration::from_secs(20)))
        .http2_keep_alive_timeout(Duration::from_secs(20))
        .http2_keep_alive_while_idle(true)
        .build()
        .map_err(|e| VideoError::Network(e.to_string()))
}

/// Formats a `reqwest::Error` with its full `std::error::Error::source()` chain so mid-stream
/// timeouts, HTTP/2 resets, and truncated chunked responses are never masked as a bare
/// `error decoding response body`.
pub(crate) fn format_network_error(err: &reqwest::Error, secrets: &[&str]) -> String {
    let mut chain = vec![err.to_string()];
    let mut cur = err.source();
    while let Some(src) = cur {
        let msg = src.to_string();
        if !msg.is_empty() && !chain.iter().any(|prev| prev.contains(&msg)) {
            chain.push(msg);
        }
        cur = src.source();
    }
    let joined = chain.join(": ");
    let labeled =
        if err.is_timeout() && !joined.to_ascii_lowercase().starts_with("request timed out") {
            format!("Request timed out: {joined}")
        } else if err.is_decode() && !err.is_timeout() {
            format!("Connection closed before full response body arrived: {joined}")
        } else {
            joined
        };
    redact_text(&labeled, secrets)
}

pub(crate) async fn collect_response_bytes(
    mut resp: reqwest::Response,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Computes exponential backoff delay when an upstream API returns HTTP 429 (rate limit)
/// or transient 502/503/504 gateway errors.
pub(crate) fn retry_delay_for_status(
    status: u16,
    headers: &HeaderMap,
    attempt: usize,
) -> Option<Duration> {
    const MAX_RETRIES: usize = 5;
    if attempt >= MAX_RETRIES || !matches!(status, 429 | 502 | 503 | 504) {
        return None;
    }
    if let Some(ra) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        return Some(Duration::from_secs(ra.clamp(2, 60)));
    }
    let secs = match attempt {
        0 => 3,
        1 => 7,
        2 => 14,
        3 => 24,
        _ => 36,
    };
    Some(Duration::from_secs(secs))
}

pub fn gemini_headers(endpoint: &StageEndpoint) -> Result<HeaderMap, VideoError> {
    let key = endpoint.api_key.trim();
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-goog-api-key",
        HeaderValue::from_str(key).map_err(|_| {
            VideoError::Config("API key contains invalid header characters.".into())
        })?,
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
            call_openai_chat(
                http,
                endpoint,
                system_prompt,
                parts,
                temperature,
                max_tokens,
            )
            .await
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

    let mut body = json!({
        "model": endpoint.model.trim(),
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": true,
    });

    let t0 = Instant::now();
    let mut attempt = 0usize;
    let text = loop {
        let send_res = http.post(&url).bearer_auth(key).json(&body).send().await;

        let resp = match send_res {
            Ok(r) => r,
            Err(_e) if attempt < 3 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                continue;
            }
            Err(e) => return Err(VideoError::Network(format_network_error(&e, &secrets))),
        };

        let status = resp.status();
        let resp_headers = resp.headers().clone();
        if let Some(delay) = retry_delay_for_status(status.as_u16(), &resp_headers, attempt) {
            attempt += 1;
            tokio::time::sleep(delay).await;
            continue;
        }
        let content_type = resp_headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let raw_bytes = match collect_response_bytes(resp).await {
            Ok(b) => b,
            Err(_e) if attempt < 3 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                continue;
            }
            Err(e) => return Err(VideoError::Network(format_network_error(&e, &secrets))),
        };
        let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

        if status.as_u16() == 400
            && raw.contains("stream")
            && body.get("stream").is_some()
            && let Some(obj) = body.as_object_mut()
        {
            obj.remove("stream");
            continue;
        }

        if !status.is_success() {
            return Err(VideoError::Provider(eddy_ai::model_failure(
                status.as_u16(),
                &content_type,
                &raw,
                &secrets,
                Some(&resp_headers),
            )));
        }

        if is_sse_payload(&content_type, &raw) {
            match extract_openai_sse_text(&raw, &secrets, Some(&resp_headers)) {
                Ok(out) => break out,
                Err(SseParseOutcome::RetryableProvider(_)) if attempt < 3 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(3 * attempt as u64)).await;
                    continue;
                }
                Err(SseParseOutcome::RetryableProvider(err) | SseParseOutcome::Fatal(err)) => {
                    return Err(err);
                }
            }
        }

        let parsed: Value = serde_json::from_str(&raw).map_err(|_| {
            VideoError::Format(format!(
                "Model endpoint returned non-JSON body: {}",
                redact_text(&raw, &secrets)
            ))
        })?;
        break extract_openai_message_text(&parsed)?;
    };

    Ok(ModelOutput {
        text,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

enum SseParseOutcome {
    RetryableProvider(VideoError),
    Fatal(VideoError),
}

fn is_sse_payload(content_type: &str, raw: &str) -> bool {
    if content_type.contains("text/event-stream") {
        return true;
    }
    let trimmed = raw.trim_start();
    trimmed.starts_with("data:") || trimmed.starts_with("event:") || trimmed.starts_with(": ")
}

fn for_each_sse_data_json<F>(raw: &str, mut on_data: F) -> Result<(), String>
where
    F: FnMut(&str, Value) -> Result<(), SseParseOutcome>,
{
    let mut pending = String::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            if !pending.trim().is_empty() {
                let chunk = std::mem::take(&mut pending);
                dispatch_sse_event(&chunk, &mut on_data)?;
            }
            continue;
        }
        if line.starts_with(':')
            || line.starts_with("event:")
            || line.starts_with("id:")
            || line.starts_with("retry:")
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let piece = rest.strip_prefix(' ').unwrap_or(rest);
            if !pending.is_empty() && serde_json::from_str::<Value>(pending.trim()).is_ok() {
                let prev = std::mem::take(&mut pending);
                dispatch_sse_event(&prev, &mut on_data)?;
            }
            if !pending.is_empty() {
                pending.push('\n');
            }
            pending.push_str(piece);
        }
    }
    if !pending.trim().is_empty() {
        dispatch_sse_event(&pending, &mut on_data)?;
    }
    Ok(())
}

fn dispatch_sse_event<F>(chunk: &str, on_data: &mut F) -> Result<(), String>
where
    F: FnMut(&str, Value) -> Result<(), SseParseOutcome>,
{
    let trimmed = chunk.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }
    let val: Value = serde_json::from_str(trimmed).map_err(|e| format!("{e}: {trimmed}"))?;
    on_data(trimmed, val).map_err(|_| String::new())
}

fn extract_openai_sse_text(
    raw: &str,
    secrets: &[&str],
    headers: Option<&HeaderMap>,
) -> Result<String, SseParseOutcome> {
    let mut out = String::new();
    let mut fatal_or_retry: Option<SseParseOutcome> = None;
    let parse_res = for_each_sse_data_json(raw, |raw_ev, val| {
        if val.get("error").is_some_and(|e| !e.is_null()) {
            let code = val["error"]["code"].as_u64().unwrap_or(502) as u16;
            let err = VideoError::Provider(eddy_ai::model_failure(
                code,
                "application/json",
                raw_ev,
                secrets,
                headers,
            ));
            let outcome = if matches!(code, 429 | 500 | 502 | 503 | 504) {
                SseParseOutcome::RetryableProvider(err)
            } else {
                SseParseOutcome::Fatal(err)
            };
            fatal_or_retry = Some(outcome);
            return Err(SseParseOutcome::Fatal(VideoError::Format(String::new())));
        }
        if let Some(choices) = val["choices"].as_array()
            && let Some(first) = choices.first()
        {
            let delta = &first["delta"]["content"];
            if let Some(s) = delta.as_str() {
                out.push_str(s);
            } else if let Some(arr) = delta.as_array() {
                for p in arr {
                    if let Some(t) = p["text"].as_str() {
                        out.push_str(t);
                    }
                }
            } else if let Some(msg_s) = first["message"]["content"].as_str() {
                out.push_str(msg_s);
            }
        }
        Ok(())
    });

    if let Some(outcome) = fatal_or_retry {
        return Err(outcome);
    }
    if let Err(bad) = parse_res
        && out.trim().is_empty()
    {
        return Err(SseParseOutcome::Fatal(VideoError::Format(format!(
            "Invalid SSE JSON from model endpoint: {}",
            redact_text(&bad, secrets)
        ))));
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        return Err(SseParseOutcome::Fatal(VideoError::Format(
            "Model returned empty streamed content.".into(),
        )));
    }
    Ok(trimmed)
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
    Err(VideoError::Format(
        "Model returned empty message content.".into(),
    ))
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
    call_gemini_generate_content_with_retries(
        http,
        endpoint,
        system_prompt,
        parts,
        temperature,
        max_tokens,
        expect_json,
        3,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn call_gemini_generate_content_with_retries(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    system_prompt: &str,
    parts: &[UserPart],
    temperature: f32,
    max_tokens: u32,
    expect_json: bool,
    max_transport_retries: usize,
) -> Result<ModelOutput, VideoError> {
    let stream_url = endpoint.gemini_stream_generate_content_url()?;
    let unary_url = endpoint.gemini_generate_content_url()?;
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
    let mut use_stream = true;
    let mut attempt = 0usize;
    let text = loop {
        let active_url = if use_stream { &stream_url } else { &unary_url };
        let send_res = http
            .post(active_url)
            .headers(headers.clone())
            .json(&payload)
            .send()
            .await;

        let resp = match send_res {
            Ok(r) => r,
            Err(_e) if attempt < max_transport_retries => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                continue;
            }
            Err(e) => return Err(VideoError::Network(format_network_error(&e, &secrets))),
        };

        let status = resp.status();
        let resp_headers = resp.headers().clone();

        // Transparently fall back to unary `:generateContent` if a custom relay or mock server
        // only exposes `:generateContent` and returns 404/405/501 on `:streamGenerateContent`.
        if use_stream && matches!(status.as_u16(), 404 | 405 | 501) {
            use_stream = false;
            continue;
        }

        if max_transport_retries > 0
            && let Some(delay) = retry_delay_for_status(status.as_u16(), &resp_headers, attempt)
        {
            attempt += 1;
            tokio::time::sleep(delay).await;
            continue;
        }

        let content_type = resp_headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let raw_bytes = match collect_response_bytes(resp).await {
            Ok(b) => b,
            Err(_e) if attempt < max_transport_retries => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                continue;
            }
            Err(e) => return Err(VideoError::Network(format_network_error(&e, &secrets))),
        };
        let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

        if status.as_u16() == 400
            && expect_json
            && raw.contains("responseMimeType")
            && payload["generationConfig"]
                .get("responseMimeType")
                .is_some()
        {
            if let Some(obj) = payload["generationConfig"].as_object_mut() {
                obj.remove("responseMimeType");
            }
            continue;
        }

        if status.as_u16() == 400
            && use_stream
            && (raw.contains("streamGenerateContent") || raw.contains("alt=sse"))
        {
            use_stream = false;
            continue;
        }

        if !status.is_success() {
            return Err(VideoError::Provider(eddy_ai::model_failure(
                status.as_u16(),
                &content_type,
                &raw,
                &secrets,
                Some(&resp_headers),
            )));
        }

        match parse_gemini_response_body(&content_type, &raw, &secrets, Some(&resp_headers)) {
            Ok(parsed_text) => break parsed_text,
            Err(SseParseOutcome::RetryableProvider(_)) if attempt < max_transport_retries => {
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(3 * attempt as u64)).await;
                continue;
            }
            Err(SseParseOutcome::RetryableProvider(err) | SseParseOutcome::Fatal(err)) => {
                return Err(err);
            }
        }
    };

    Ok(ModelOutput {
        text,
        elapsed_ms: t0.elapsed().as_millis() as u64,
    })
}

fn append_gemini_candidate_parts(
    val: &Value,
    answer_buf: &mut String,
    thought_buf: &mut String,
) -> bool {
    let Some(cands) = val["candidates"].as_array() else {
        return false;
    };
    let Some(first) = cands.first() else {
        return false;
    };
    if let Some(parts) = first["content"]["parts"].as_array() {
        for p in parts {
            if let Some(t) = p["text"].as_str()
                && !t.is_empty()
            {
                if p["thought"].as_bool() == Some(true) {
                    thought_buf.push_str(t);
                } else {
                    answer_buf.push_str(t);
                }
            }
        }
    }
    true
}

fn parse_gemini_response_body(
    content_type: &str,
    raw: &str,
    secrets: &[&str],
    headers: Option<&HeaderMap>,
) -> Result<String, SseParseOutcome> {
    if is_sse_payload(content_type, raw) {
        let mut answer_buf = String::new();
        let mut thought_buf = String::new();
        let mut saw_candidate = false;
        let mut fatal_or_retry: Option<SseParseOutcome> = None;

        let parse_res = for_each_sse_data_json(raw, |raw_ev, val| {
            if val.get("error").is_some_and(|e| !e.is_null()) {
                let code = val["error"]["code"].as_u64().unwrap_or(502) as u16;
                let err = VideoError::Provider(eddy_ai::model_failure(
                    code,
                    "application/json",
                    raw_ev,
                    secrets,
                    headers,
                ));
                let outcome = if matches!(code, 429 | 500 | 502 | 503 | 504) {
                    SseParseOutcome::RetryableProvider(err)
                } else {
                    SseParseOutcome::Fatal(err)
                };
                fatal_or_retry = Some(outcome);
                return Err(SseParseOutcome::Fatal(VideoError::Format(String::new())));
            }
            if append_gemini_candidate_parts(&val, &mut answer_buf, &mut thought_buf) {
                saw_candidate = true;
            }
            Ok(())
        });

        if let Some(outcome) = fatal_or_retry {
            return Err(outcome);
        }
        if let Err(bad) = parse_res
            && answer_buf.trim().is_empty()
            && thought_buf.trim().is_empty()
        {
            return Err(SseParseOutcome::Fatal(VideoError::Format(format!(
                "Gemini SSE stream returned invalid JSON: {}",
                redact_text(&bad, secrets)
            ))));
        }
        let chosen = if !answer_buf.trim().is_empty() {
            answer_buf.trim().to_string()
        } else {
            thought_buf.trim().to_string()
        };
        if !chosen.is_empty() {
            return Ok(chosen);
        }
        if !saw_candidate {
            return Err(SseParseOutcome::Fatal(VideoError::Provider(format!(
                "Gemini endpoint returned no candidates: {}",
                redact_text(raw, secrets)
            ))));
        }
        return Err(SseParseOutcome::Fatal(VideoError::Format(
            "Gemini response contained no text output.".into(),
        )));
    }

    let parsed: Value = serde_json::from_str(raw).map_err(|_| {
        SseParseOutcome::Fatal(VideoError::Format(format!(
            "Gemini endpoint returned non-JSON body: {}",
            redact_text(raw, secrets)
        )))
    })?;

    if parsed.get("error").is_some_and(|e| !e.is_null()) {
        let code = parsed["error"]["code"].as_u64().unwrap_or(502) as u16;
        let err = VideoError::Provider(eddy_ai::model_failure(
            code,
            content_type,
            raw,
            secrets,
            headers,
        ));
        return if matches!(code, 429 | 500 | 502 | 503 | 504) {
            Err(SseParseOutcome::RetryableProvider(err))
        } else {
            Err(SseParseOutcome::Fatal(err))
        };
    }

    if let Some(arr) = parsed.as_array() {
        let mut answer_buf = String::new();
        let mut thought_buf = String::new();
        let mut saw_candidate = false;
        for item in arr {
            if append_gemini_candidate_parts(item, &mut answer_buf, &mut thought_buf) {
                saw_candidate = true;
            }
        }
        let chosen = if !answer_buf.trim().is_empty() {
            answer_buf.trim().to_string()
        } else {
            thought_buf.trim().to_string()
        };
        if !chosen.is_empty() {
            return Ok(chosen);
        }
        return Err(SseParseOutcome::Fatal(if saw_candidate {
            VideoError::Format("Gemini response contained no text output.".into())
        } else {
            VideoError::Provider(format!(
                "Gemini endpoint returned no candidates: {}",
                redact_text(raw, secrets)
            ))
        }));
    }

    let candidates = parsed["candidates"]
        .as_array()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            SseParseOutcome::Fatal(VideoError::Provider(format!(
                "Gemini endpoint returned no candidates: {}",
                redact_text(raw, secrets)
            )))
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
        return Err(SseParseOutcome::Fatal(VideoError::Format(
            "Gemini response contained no text output.".into(),
        )));
    }
    Ok(text)
}

/// Tests connectivity and real model invocation for any stage endpoint (`Stage 1`, `VLM`, `Stage 2`, or `ASR`).
pub async fn test_stage_endpoint(
    endpoint: &StageEndpoint,
    is_asr_role: bool,
) -> Result<u64, VideoError> {
    let http = build_streaming_http_client(45, 90)?;
    let model_lower = endpoint.model.to_ascii_lowercase();
    if is_asr_role
        || model_lower.contains("asr")
        || model_lower.contains("whisper")
        || model_lower.contains("transcribe")
    {
        return crate::asr::probe_asr_endpoint(&http, endpoint).await;
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
