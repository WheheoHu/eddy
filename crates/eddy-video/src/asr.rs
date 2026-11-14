use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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
use crate::client::{
    UserPart, build_streaming_http_client, call_gemini_generate_content, collect_response_bytes,
    format_network_error, retry_delay_for_status,
};
use crate::media::{merge_micro_segments, run_cmd};
use crate::models::{TranscriptBundle, TranscriptSegment};
use crate::redact::redact_text;
use crate::settings::{ResolvedProtocol, StageEndpoint};

static SILENCE_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"silence_start:\s*([0-9]+(?:\.[0-9]+)?)").unwrap());
static SILENCE_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"silence_end:\s*([0-9]+(?:\.[0-9]+)?)").unwrap());

/// Generates a valid 0.45-second 8kHz 16-bit mono PCM WAV (~7.2 KB) in memory so `Test`
/// genuinely invokes the configured ASR model without needing external files.
pub fn tiny_test_wav_bytes() -> Vec<u8> {
    let sample_rate: u32 = 8_000;
    let num_samples: u32 = 3_600; // 0.45s
    let data_len: u32 = num_samples * 2;
    let riff_len: u32 = 36 + data_len;

    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_len.to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // Mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // Byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // Block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..num_samples {
        // Subtle 440Hz faint tone in the middle 100ms so VAD-strict ASR endpoints accept the frame
        let sample: i16 = if (1_200..2_000).contains(&i) {
            let phase = (i as f32 * std::f32::consts::TAU * 440.0) / sample_rate as f32;
            (phase.sin() * 600.0) as i16
        } else {
            0
        };
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    buf
}

/// Sends a real audio inference request to verify ASR endpoint URL, model name, key, and rate-limit status.
pub async fn probe_asr_endpoint(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
) -> Result<u64, VideoError> {
    if !endpoint.is_configured() {
        return Err(VideoError::Config(
            "ASR endpoint is missing URL, model ID, or API key.".into(),
        ));
    }
    let wav = tiny_test_wav_bytes();
    let t0 = Instant::now();
    if endpoint.resolved_protocol() == ResolvedProtocol::Gemini {
        let _ = call_gemini_generate_content(
            http,
            endpoint,
            "",
            &[
                UserPart::InlineMedia {
                    mime: "audio/wav".into(),
                    bytes: wav,
                },
                UserPart::Text("Transcribe any speech in this audio clip, or reply OK if silent.".into()),
            ],
            0.0,
            128,
            false,
        )
        .await?;
        return Ok(t0.elapsed().as_millis() as u64);
    }

    let model_lower = endpoint.model.to_ascii_lowercase();
    if model_lower.contains("whisper")
        || model_lower.contains("transcribe")
        || endpoint.url.trim_end_matches('/').ends_with("/audio/transcriptions")
    {
        let _ = call_openai_audio_transcriptions(
            http,
            endpoint,
            "probe.wav",
            "audio/wav",
            wav,
            0,
            450,
        )
        .await?;
    } else {
        let _ = call_openai_chat_input_audio(
            http,
            endpoint,
            &wav,
            "audio/wav",
            "wav",
            0,
            450,
        )
        .await?;
    }
    Ok(t0.elapsed().as_millis() as u64)
}

/// Transcribes `audio_path` using ~110s natural-pause VAD windows (staying well under 60 RPM
/// rate limits on long 80-minute lectures), per-chunk checkpoints, and automatic 429 backoff.
#[allow(clippy::too_many_arguments)]
pub async fn transcribe_with_api(
    ffmpeg: &str,
    ffprobe: &str,
    audio_path: &Path,
    video_id: &str,
    endpoint: &StageEndpoint,
    work_dir: &Path,
    context_hint: &str,
    on_chunk_progress: Option<Arc<dyn Fn(usize, usize) + Send + Sync>>,
) -> Result<TranscriptBundle, VideoError> {
    if !endpoint.is_configured() {
        return Err(VideoError::Config(
            "No subtitles found and Speech Recognition (ASR) endpoint is not configured in Settings -> Video."
                .into(),
        ));
    }

    let chunks_dir = work_dir.join("vad_chunks_v2");
    std::fs::create_dir_all(&chunks_dir)?;
    // Remove legacy 32s chunk directory if left over from an earlier failed run
    let _ = std::fs::remove_dir_all(work_dir.join("vad_chunks"));

    // ~110s target (max 165s, min 40s): well within Qwen3-ASR-Flash's 5-min / 10MB limit,
    // reducing an 80-minute lecture from ~150 calls down to ~42 calls.
    let windows = plan_vad_windows_ms(ffmpeg, ffprobe, audio_path, 110_000, 165_000, 40_000).await?;
    let total_chunks = windows.len();
    let sliced = slice_audio_windows(ffmpeg, audio_path, &windows, &chunks_dir).await?;

    let http = build_streaming_http_client(120, 360)?;
    let sem = Arc::new(Semaphore::new(2));
    let done_count = Arc::new(AtomicUsize::new(0));
    let mut futs = FuturesUnordered::new();

    for (idx, (start_ms, end_ms, chunk_path)) in sliced.into_iter().enumerate() {
        let sem = sem.clone();
        let http = http.clone();
        let ep = endpoint.clone();
        let hint = context_hint.to_string();
        let cache_json = chunks_dir.join(format!("vad_{idx:04}_{start_ms:08}_{end_ms:08}.json"));
        let done_ref = done_count.clone();
        let prog_cb = on_chunk_progress.clone();

        futs.push(async move {
            if cache_json.is_file()
                && let Ok(bytes) = std::fs::read(&cache_json)
                && let Ok(cached_segs) = serde_json::from_slice::<Vec<TranscriptSegment>>(&bytes)
            {
                let done = done_ref.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(cb) = &prog_cb {
                    cb(done, total_chunks);
                }
                return Ok::<(usize, Vec<TranscriptSegment>), VideoError>((idx, cached_segs));
            }

            let _permit = sem.acquire_owned().await.unwrap();
            let segs = transcribe_single_chunk(&http, &ep, &chunk_path, start_ms, end_ms, &hint).await?;
            let _ = std::fs::write(&cache_json, serde_json::to_vec(&segs).unwrap_or_default());

            let done = done_ref.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(cb) = &prog_cb {
                cb(done, total_chunks);
            }
            Ok((idx, segs))
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
            4096,
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
        let fname = chunk_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("chunk.mp3");
        return call_openai_audio_transcriptions(
            http,
            endpoint,
            fname,
            "audio/mpeg",
            raw_bytes,
            start_ms,
            end_ms,
        )
        .await;
    }

    call_openai_chat_input_audio(
        http,
        endpoint,
        &raw_bytes,
        "audio/mpeg",
        "mp3",
        start_ms,
        end_ms,
    )
    .await
}

async fn call_openai_chat_input_audio(
    http: &reqwest::Client,
    endpoint: &StageEndpoint,
    raw_bytes: &[u8],
    mime_type: &str,
    format_tag: &str,
    start_ms: u64,
    end_ms: u64,
) -> Result<Vec<TranscriptSegment>, VideoError> {
    let base = endpoint.normalized_openai_base_url()?;
    let url = format!("{base}/chat/completions");
    let key = endpoint.api_key.trim();
    let secrets = [key];
    let b64 = BASE64.encode(raw_bytes);
    let data_uri = format!("data:{mime_type};base64,{b64}");
    let model_lower = endpoint.model.to_ascii_lowercase();

    let mut payload = if model_lower.contains("asr") {
        // Qwen3-ASR-Flash OpenAI-compatible spec: single `user` message with `input_audio` Data URI
        json!({
            "model": endpoint.model.trim(),
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "input_audio",
                    "input_audio": { "data": data_uri, "format": format_tag }
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
                    { "type": "input_audio", "input_audio": { "data": b64, "format": format_tag } },
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
                    { "type": "input_audio", "input_audio": { "data": data_uri, "format": format_tag } },
                    { "type": "text", "text": "Transcribe the audio verbatim with punctuation, preserving technical terms. Output only the transcript." }
                ]
            }],
            "temperature": 0.0
        })
    };

    let mut attempt = 0usize;
    let (status, headers, ct, raw) = loop {
        let send_res = http
            .post(&url)
            .bearer_auth(key)
            .json(&payload)
            .send()
            .await;

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
        let headers = resp.headers().clone();
        if let Some(delay) = retry_delay_for_status(status.as_u16(), &headers, attempt) {
            attempt += 1;
            tokio::time::sleep(delay).await;
            continue;
        }
        if status.as_u16() == 400
            && payload.get("asr_options").is_some()
            && let Some(obj) = payload.as_object_mut()
        {
            obj.remove("asr_options");
            continue;
        }

        let ct = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
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
        break (status, headers, ct, raw);
    };

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
    file_name: &str,
    mime_type: &str,
    raw_bytes: Vec<u8>,
    start_ms: u64,
    end_ms: u64,
) -> Result<Vec<TranscriptSegment>, VideoError> {
    let base = endpoint.normalized_openai_base_url()?;
    let url = format!("{base}/audio/transcriptions");
    let key = endpoint.api_key.trim();
    let secrets = [key];

    let mut attempt = 0usize;
    let (status, headers, ct, raw) = loop {
        let file_part = multipart::Part::bytes(raw_bytes.clone())
            .file_name(file_name.to_string())
            .mime_str(mime_type)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        let form = multipart::Form::new()
            .text("model", endpoint.model.trim().to_string())
            .text("response_format", "json")
            .part("file", file_part);

        let send_res = http
            .post(&url)
            .bearer_auth(key)
            .multipart(form)
            .send()
            .await;

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
        let headers = resp.headers().clone();
        if let Some(delay) = retry_delay_for_status(status.as_u16(), &headers, attempt) {
            attempt += 1;
            tokio::time::sleep(delay).await;
            continue;
        }
        let ct = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
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
        break (status, headers, ct, raw);
    };

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

    let bin_parent = Path::new(ffmpeg).parent().unwrap_or_else(|| Path::new(""));
    let child_path = crate::toolchain::augmented_child_path(&[bin_parent]);
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
        .env("PATH", child_path)
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
        if !chunk_path.is_file() {
            let start_s = format!("{:.3}", start_ms as f64 / 1000.0);
            let dur_s =
                format!("{:.3}", (end_ms.saturating_sub(start_ms)).max(200) as f64 / 1000.0);
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
                    "32k",
                    &out_str,
                ],
                &[],
            )
            .await?;
        }
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
