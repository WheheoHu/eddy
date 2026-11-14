use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use reqwest::multipart;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::VideoError;
use crate::client::{UserPart, build_http_client, call_gemini_generate_content};
use crate::media::{merge_micro_segments, run_cmd};
use crate::models::{TranscriptBundle, TranscriptSegment};
use crate::redact::redact_text;
use crate::settings::{ResolvedProtocol, StageEndpoint};

static SILENCE_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"silence_start:\s*([0-9]+(?:\.[0-9]+)?)").unwrap());
static SILENCE_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"silence_end:\s*([0-9]+(?:\.[0-9]+)?)").unwrap());

/// Transcribes `audio_path` using local `ffmpeg silencedetect` VAD windows + the configured ASR endpoint.
pub async fn transcribe_with_api(
    ffmpeg: &str,
    ffprobe: &str,
    audio_path: &Path,
    video_id: &str,
    endpoint: &StageEndpoint,
    work_dir: &Path,
    context_hint: &str,
) -> Result<TranscriptBundle, VideoError> {
    if !endpoint.is_configured() {
        return Err(VideoError::Config(
            "No subtitles found and Speech Recognition (ASR) endpoint is not configured in Settings -> Video."
                .into(),
        ));
    }

    let chunks_dir = work_dir.join("vad_chunks");
    std::fs::create_dir_all(&chunks_dir)?;

    let windows = plan_vad_windows_ms(ffmpeg, ffprobe, audio_path, 32_000, 75_000, 12_000).await?;
    let sliced = slice_audio_windows(ffmpeg, audio_path, &windows, &chunks_dir).await?;

    let http = build_http_client(180)?;
    let sem = Arc::new(Semaphore::new(3));
    let mut futs = FuturesUnordered::new();

    for (idx, (start_ms, end_ms, chunk_path)) in sliced.into_iter().enumerate() {
        let sem = sem.clone();
        let http = http.clone();
        let ep = endpoint.clone();
        let hint = context_hint.to_string();
        futs.push(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            let segs = transcribe_single_chunk(&http, &ep, &chunk_path, start_ms, end_ms, &hint).await?;
            Ok::<(usize, Vec<TranscriptSegment>), VideoError>((idx, segs))
        });
    }

    let mut ordered: Vec<(usize, Vec<TranscriptSegment>)> = Vec::new();
    while let Some(res) = futs.next().await {
        ordered.push(res?);
    }
    ordered.sort_by_key(|x| x.0);

    let mut flat = Vec::new();
    for (_, segs) in ordered {
        for mut s in segs {
            s.index = flat.len();
            flat.push(s);
        }
    }
    let _ = std::fs::remove_dir_all(&chunks_dir);

    Ok(TranscriptBundle {
        video_id: video_id.to_string(),
        source: "asr_api".into(),
        language: "auto".into(),
        segments: merge_micro_segments(flat),
    })
}

async fn transcribe_single_chunk(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    chunk_path: &Path,
    start_ms: u64,
    end_ms: u64,
    context_hint: &str,
) -> Result<Vec<TranscriptSegment>, VideoError> {
    let raw_bytes = std::fs::read(chunk_path)?;
    if raw_bytes.is_empty() {
        return Ok(Vec::new());
    }

    if endpoint.resolved_protocol() == ResolvedProtocol::Gemini {
        let mut prompt = String::from(
            "Transcribe this speech audio verbatim in its original spoken language(s), \
             preserving technical terms and adding natural punctuation. \
             Output ONLY the transcribed speech text without timestamps or commentary.",
        );
        if !context_hint.trim().is_empty() {
            prompt.push_str("\nTopic hint: ");
            prompt.push_str(context_hint.trim());
        }
        let out = call_gemini_generate_content(
            http,
            endpoint,
            "",
            &[
                UserPart::InlineMedia {
                    mime: "audio/mp3".into(),
                    bytes: raw_bytes,
                },
                UserPart::Text(prompt),
            ],
            0.0,
            2048,
            false,
        )
        .await?;
        return Ok(interpolate_sentences(&out.text, start_ms, end_ms));
    }

    let model_lower = endpoint.model.to_ascii_lowercase();
    if model_lower.contains("whisper")
        || model_lower.contains("transcribe")
        || endpoint.url.trim_end_matches('/').ends_with("/audio/transcriptions")
    {
        return call_openai_audio_transcriptions(http, endpoint, chunk_path, raw_bytes, start_ms, end_ms).await;
    }

    call_openai_chat_input_audio(http, endpoint, &raw_bytes, start_ms, end_ms).await
}

async fn call_openai_chat_input_audio(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    raw_bytes: &[u8],
    start_ms: u64,
    end_ms: u64,
) -> Result<Vec<TranscriptSegment>, VideoError> {
    let base = endpoint.normalized_openai_base_url()?;
    let url = format!("{base}/chat/completions");
    let key = endpoint.api_key.trim();
    let secrets = [key];
    let b64 = BASE64.encode(raw_bytes);
    let data_uri = format!("data:audio/mpeg;base64,{b64}");
    let model_lower = endpoint.model.to_ascii_lowercase();

    let mut payload = if model_lower.contains("asr") {
        // Qwen3-ASR-Flash OpenAI-compatible spec: single `user` message with `input_audio` Data URI
        json!({
            "model": endpoint.model.trim(),
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "input_audio",
                    "input_audio": { "data": data_uri, "format": "mp3" }
                }]
            }],
            "stream": false,
            "asr_options": { "enable_itn": true }
        })
    } else if model_lower.contains("gpt-") {
        json!({
            "model": endpoint.model.trim(),
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "input_audio", "input_audio": { "data": b64, "format": "mp3" } },
                    { "type": "text", "text": "Transcribe the audio verbatim with punctuation. Output only the transcript." }
                ]
            }],
            "temperature": 0.0
        })
    } else {
        json!({
            "model": endpoint.model.trim(),
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "input_audio", "input_audio": { "data": data_uri, "format": "mp3" } },
                    { "type": "text", "text": "Transcribe the audio verbatim with punctuation, preserving technical terms. Output only the transcript." }
                ]
            }],
            "temperature": 0.0
        })
    };

    let mut resp = http
        .post(&url)
        .bearer_auth(key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    if resp.status().as_u16() == 400
        && payload.get("asr_options").is_some()
        && let Some(obj) = payload.as_object_mut()
    {
        obj.remove("asr_options");
        resp = http
            .post(&url)
            .bearer_auth(key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;
    }

    let status = resp.status();
    let headers = resp.headers().clone();
    let ct = headers
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
            &ct,
            &raw,
            &secrets,
            Some(&headers),
        )));
    }

    let parsed: Value = serde_json::from_str(&raw).map_err(|_| {
        VideoError::Format(format!(
            "ASR endpoint returned non-JSON body: {}",
            redact_text(&raw, &secrets)
        ))
    })?;
    let text = parsed["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim();
    Ok(interpolate_sentences(text, start_ms, end_ms))
}

async fn call_openai_audio_transcriptions(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    chunk_path: &Path,
    raw_bytes: Vec<u8>,
    start_ms: u64,
    end_ms: u64,
) -> Result<Vec<TranscriptSegment>, VideoError> {
    let base = endpoint.normalized_openai_base_url()?;
    let url = format!("{base}/audio/transcriptions");
    let key = endpoint.api_key.trim();
    let secrets = [key];
    let fname = chunk_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("chunk.mp3")
        .to_string();

    let file_part = multipart::Part::bytes(raw_bytes)
        .file_name(fname)
        .mime_str("audio/mpeg")
        .map_err(|e| VideoError::Format(e.to_string()))?;
    let form = multipart::Form::new()
        .text("model", endpoint.model.trim().to_string())
        .text("response_format", "json")
        .part("file", file_part);

    let resp = http
        .post(&url)
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| VideoError::Network(redact_text(&e.to_string(), &secrets)))?;

    let status = resp.status();
    let headers = resp.headers().clone();
    let ct = headers
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
            &ct,
            &raw,
            &secrets,
            Some(&headers),
        )));
    }

    let parsed: Value = serde_json::from_str(&raw).unwrap_or_default();
    let text = parsed["text"].as_str().unwrap_or("").trim();
    Ok(interpolate_sentences(text, start_ms, end_ms))
}

async fn plan_vad_windows_ms(
    ffmpeg: &str,
    ffprobe: &str,
    audio_path: &Path,
    target_ms: u64,
    max_ms: u64,
    min_ms: u64,
) -> Result<Vec<(u64, u64)>, VideoError> {
    let audio_str = audio_path.to_string_lossy().into_owned();
    let (dur_out, _) = run_cmd(
        ffprobe,
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            &audio_str,
        ],
        &[],
    )
    .await?;
    let duration_ms = dur_out
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| (v * 1000.0).round() as u64)
        .unwrap_or(60_000);

    if duration_ms <= max_ms {
        return Ok(vec![(0, duration_ms)]);
    }

    let output = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
            &audio_str,
            "-af",
            "silencedetect=noise=-32dB:d=0.4",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    let starts: Vec<f64> = SILENCE_START_RE
        .captures_iter(&stderr)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    let ends: Vec<f64> = SILENCE_END_RE
        .captures_iter(&stderr)
        .filter_map(|c| c[1].parse().ok())
        .collect();

    let mut pauses: Vec<u64> = starts
        .into_iter()
        .zip(ends)
        .map(|(s, e)| (((s + e) * 0.5) * 1000.0).round() as u64)
        .filter(|&m| m > 500 && m + 500 < duration_ms)
        .collect();
    pauses.sort_unstable();
    pauses.dedup();

    let mut windows = Vec::new();
    let mut cursor = 0u64;
    while cursor < duration_ms {
        if duration_ms - cursor <= max_ms {
            windows.push((cursor, duration_ms));
            break;
        }
        let ideal = cursor + target_ms;
        let cut = pauses
            .iter()
            .copied()
            .filter(|&p| p >= cursor + min_ms && p <= cursor + max_ms)
            .min_by_key(|&p| p.abs_diff(ideal))
            .unwrap_or(ideal);
        windows.push((cursor, cut));
        cursor = cut;
    }
    Ok(windows)
}

async fn slice_audio_windows(
    ffmpeg: &str,
    audio_path: &Path,
    windows: &[(u64, u64)],
    out_dir: &Path,
) -> Result<Vec<(u64, u64, PathBuf)>, VideoError> {
    if let [(0, end_ms)] = windows {
        return Ok(vec![(0, *end_ms, audio_path.to_path_buf())]);
    }
    let in_str = audio_path.to_string_lossy().into_owned();
    let mut out = Vec::with_capacity(windows.len());
    for (idx, &(start_ms, end_ms)) in windows.iter().enumerate() {
        let chunk_path = out_dir.join(format!("vad_{idx:04}_{start_ms:08}_{end_ms:08}.mp3"));
        let start_s = format!("{:.3}", start_ms as f64 / 1000.0);
        let dur_s = format!("{:.3}", (end_ms.saturating_sub(start_ms)).max(200) as f64 / 1000.0);
        let out_str = chunk_path.to_string_lossy().into_owned();
        run_cmd(
            ffmpeg,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-ss",
                &start_s,
                "-t",
                &dur_s,
                "-i",
                &in_str,
                "-vn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "48k",
                &out_str,
            ],
            &[],
        )
        .await?;
        out.push((start_ms, end_ms, chunk_path));
    }
    Ok(out)
}

pub fn interpolate_sentences(text: &str, start_ms: u64, end_ms: u64) -> Vec<TranscriptSegment> {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return Vec::new();
    }
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in cleaned.chars() {
        cur.push(ch);
        if matches!(ch, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';') {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                sentences.push(s);
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        sentences.push(cur.trim().to_string());
    }
    if sentences.len() <= 1 {
        return vec![TranscriptSegment {
            index: 0,
            start_ms,
            end_ms: end_ms.max(start_ms + 200),
            text: cleaned.to_string(),
        }];
    }

    let total_chars: usize = sentences.iter().map(|s| s.chars().count()).sum::<usize>().max(1);
    let span_ms = end_ms.saturating_sub(start_ms).max(sentences.len() as u64 * 100);
    let mut cursor = start_ms;
    let mut out = Vec::with_capacity(sentences.len());
    let last_idx = sentences.len() - 1;
    for (i, sent) in sentences.into_iter().enumerate() {
        let next_ms = if i == last_idx {
            end_ms.max(cursor + 100)
        } else {
            let share = ((span_ms as f64) * (sent.chars().count() as f64 / total_chars as f64)).round() as u64;
            (cursor + share.max(100)).min(end_ms)
        };
        out.push(TranscriptSegment {
            index: i,
            start_ms: cursor,
            end_ms: next_ms,
            text: sent,
        });
        cursor = next_ms;
    }
    out
}
