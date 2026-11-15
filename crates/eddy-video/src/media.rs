use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::VideoError;
use crate::models::{
    CandidateFrame, ChapterHint, TranscriptBundle, TranscriptSegment, VideoProbe,
    parse_timestamp_to_ms,
};
use crate::redact::redact_text;
use crate::settings::TranscriptPolicy;
use crate::toolchain::ToolchainStatus;

static YOUTUBE_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:v=|youtu\.be/|shorts/|embed/)([A-Za-z0-9_-]{11})").unwrap());
static VTT_TIMING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*((?:\d{1,2}:)?\d{2}:\d{2}[.,]\d{3})\s*-->\s*((?:\d{1,2}:)?\d{2}:\d{2}[.,]\d{3})",
    )
    .unwrap()
});
static INLINE_TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static PTS_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"pts_time:\s*([0-9]+(?:\.[0-9]+)?)").unwrap());

fn sha_short(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    let out = h.finalize();
    out.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

pub fn compute_video_id(source: &str) -> String {
    let p = Path::new(source.trim());
    if p.exists() {
        let size = p.metadata().map(|m| m.len()).unwrap_or(0);
        let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let stem: String = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(20)
            .collect();
        return format!(
            "{stem}_{}",
            sha_short(&format!("{}:{size}", canon.display()))
        );
    }
    if let Some(cap) = YOUTUBE_ID_RE.captures(source) {
        return format!("yt_{}", &cap[1]);
    }
    format!("url_{}", sha_short(source.trim()))
}

pub(crate) async fn run_cmd(
    bin: &str,
    args: &[&str],
    secrets: &[&str],
) -> Result<(String, String), VideoError> {
    let bin_parent = Path::new(bin).parent().unwrap_or_else(|| Path::new(""));
    let child_path = crate::toolchain::augmented_child_path(&[bin_parent]);
    let output = Command::new(bin)
        .args(args)
        .env("PATH", child_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| VideoError::Toolchain(format!("Failed to launch `{bin}`: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        let safe = redact_text(detail, secrets);
        return Err(VideoError::Media(format!(
            "`{}` exited with {}: {}",
            Path::new(bin)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(bin),
            output.status,
            safe
        )));
    }
    Ok((stdout, stderr))
}

/// Probes metadata, duration, chapters, and native subtitle availability for a URL or local file.
pub async fn probe_video(
    source: &str,
    toolchain: &ToolchainStatus,
    cookies_from_browser: Option<&str>,
) -> Result<VideoProbe, VideoError> {
    let trimmed = source.trim();
    let local_path = Path::new(trimmed);
    if local_path.is_file() {
        return probe_local_file(local_path, toolchain).await;
    }
    probe_remote_url(trimmed, toolchain, cookies_from_browser).await
}

async fn probe_local_file(
    path: &Path,
    toolchain: &ToolchainStatus,
) -> Result<VideoProbe, VideoError> {
    let ffprobe = toolchain.ffprobe_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain("`ffprobe` is not installed (install `ffmpeg` via Homebrew).".into())
    })?;
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = canon.to_string_lossy().into_owned();

    let (stdout, _) = run_cmd(
        ffprobe,
        &[
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
            &path_str,
        ],
        &[],
    )
    .await?;

    let info: Value = serde_json::from_str(&stdout).unwrap_or_default();
    let duration_sec = info["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let tags = &info["format"]["tags"];
    let fallback_title = canon
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Local Presentation")
        .to_string();
    let title = tags["title"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or(fallback_title);
    let author = tags["artist"]
        .as_str()
        .or_else(|| tags["author"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let mut chapters = Vec::new();
    if let Some(arr) = info["chapters"].as_array() {
        for (i, ch) in arr.iter().enumerate() {
            let start_ms = ch["start_time"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| (v * 1000.0).round() as u64)
                .unwrap_or(0);
            let end_ms = ch["end_time"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| (v * 1000.0).round() as u64)
                .unwrap_or(start_ms);
            let ch_title = ch["tags"]["title"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("Chapter {}", i + 1));
            chapters.push(ChapterHint {
                title: ch_title,
                start_ms,
                end_ms,
            });
        }
    }

    let mut manual_sub_langs = Vec::new();
    for ext in ["json3", "vtt", "srt"] {
        if canon.with_extension(ext).is_file() {
            manual_sub_langs.push(format!("sidecar:{ext}"));
        }
    }
    if let Some(streams) = info["streams"].as_array() {
        for st in streams {
            if st["codec_type"].as_str() == Some("subtitle") {
                let lang = st["tags"]["language"]
                    .as_str()
                    .unwrap_or("embedded")
                    .to_string();
                manual_sub_langs.push(format!("embedded:{lang}"));
            }
        }
    }

    Ok(VideoProbe {
        video_id: compute_video_id(&path_str),
        source_url: None,
        local_video_path: Some(path_str),
        is_local_file: true,
        title,
        author,
        duration_ms: (duration_sec.max(0.0) * 1000.0).round() as u64,
        chapters,
        manual_sub_langs,
        auto_sub_langs: Vec::new(),
    })
}

async fn probe_remote_url(
    url: &str,
    toolchain: &ToolchainStatus,
    cookies_from_browser: Option<&str>,
) -> Result<VideoProbe, VideoError> {
    let yt_dlp = toolchain.yt_dlp_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain(
            "`yt-dlp` is not installed (install via `brew install yt-dlp`).".into(),
        )
    })?;
    let mut args = vec!["--dump-single-json", "--no-warnings", "--no-playlist"];
    if let Some(browser) = cookies_from_browser.filter(|s| !s.trim().is_empty()) {
        args.push("--cookies-from-browser");
        args.push(browser);
    }
    args.push(url);

    let (stdout, _) = match run_cmd(yt_dlp, &args, &[]).await {
        Ok(res) => res,
        Err(_) if cookies_from_browser.is_some() => {
            // Retry without cookies if a logged-in YouTube cookie jar triggered a format/n-sig mismatch
            run_cmd(
                yt_dlp,
                &["--dump-single-json", "--no-warnings", "--no-playlist", url],
                &[],
            )
            .await?
        }
        Err(e) => return Err(e),
    };
    let raw: Value = serde_json::from_str(&stdout)
        .map_err(|e| VideoError::Media(format!("Invalid yt-dlp metadata JSON: {e}")))?;

    let mut chapters = Vec::new();
    if let Some(arr) = raw["chapters"].as_array() {
        for (i, ch) in arr.iter().enumerate() {
            let start_ms = (ch["start_time"].as_f64().unwrap_or(0.0) * 1000.0).round() as u64;
            let end_ms = (ch["end_time"].as_f64().unwrap_or(0.0) * 1000.0).round() as u64;
            let title = ch["title"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("Section {}", i + 1));
            chapters.push(ChapterHint {
                title,
                start_ms,
                end_ms,
            });
        }
    }

    let manual_sub_langs: Vec<String> = raw["subtitles"]
        .as_object()
        .map(|obj| {
            obj.keys()
                .filter(|k| k.as_str() != "live_chat")
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Filter out YouTube `tlang=` auto-translated tracks to avoid HTTP 429 & mangled terms
    let mut auto_sub_langs = Vec::new();
    if let Some(obj) = raw["automatic_captions"].as_object() {
        for (lang, fmts) in obj {
            if lang == "live_chat" {
                continue;
            }
            let is_native = fmts.as_array().is_some_and(|arr| {
                arr.iter()
                    .any(|f| f["url"].as_str().is_some_and(|u| !u.contains("tlang=")))
            });
            if is_native {
                auto_sub_langs.push(lang.clone());
            }
        }
        if auto_sub_langs.is_empty() {
            auto_sub_langs = obj
                .keys()
                .filter(|k| k.as_str() != "live_chat")
                .cloned()
                .collect();
        }
    }

    let raw_id = raw["id"].as_str().unwrap_or_default();
    let video_id = if !raw_id.is_empty() && url.contains("youtu") {
        format!("yt_{raw_id}")
    } else {
        compute_video_id(url)
    };
    let title = raw["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&video_id)
        .to_string();
    let author = raw["uploader"]
        .as_str()
        .or_else(|| raw["channel"].as_str())
        .map(String::from);
    let duration_ms = (raw["duration"].as_f64().unwrap_or(0.0).max(0.0) * 1000.0).round() as u64;
    let source_url = raw["webpage_url"].as_str().unwrap_or(url).to_string();

    Ok(VideoProbe {
        video_id,
        source_url: Some(source_url),
        local_video_path: None,
        is_local_file: false,
        title,
        author,
        duration_ms,
        chapters,
        manual_sub_langs,
        auto_sub_langs,
    })
}

fn pick_best_sub_lang(available: &[String]) -> Option<String> {
    if available.is_empty() {
        return None;
    }
    if let Some(orig) = available
        .iter()
        .find(|k| k.to_ascii_lowercase().ends_with("-orig"))
    {
        return Some(orig.clone());
    }
    for pref in [
        "zh-Hans", "zh-CN", "zh", "zh-Hant", "zh-TW", "en-orig", "en", "en-US", "ja",
    ] {
        if let Some(found) = available.iter().find(|a| a.eq_ignore_ascii_case(pref)) {
            return Some(found.clone());
        }
    }
    Some(available[0].clone())
}

/// Attempts to obtain cleaned subtitles according to `policy`. Returns `Ok(None)` when ASR should run.
pub async fn try_fetch_subtitles(
    probe: &VideoProbe,
    work_dir: &Path,
    policy: TranscriptPolicy,
    toolchain: &ToolchainStatus,
    cookies_from_browser: Option<&str>,
) -> Result<Option<TranscriptBundle>, VideoError> {
    if policy == TranscriptPolicy::AlwaysAsr {
        return Ok(None);
    }

    let subs_dir = work_dir.join("subs");
    std::fs::create_dir_all(&subs_dir)?;

    if probe.is_local_file
        && let Some(vpath_str) = &probe.local_video_path
    {
        let vpath = Path::new(vpath_str);
        for ext in ["json3", "vtt", "srt"] {
            let sidecar = vpath.with_extension(ext);
            if sidecar.is_file() {
                let segs = parse_subtitle_file(&sidecar)?;
                if !segs.is_empty() {
                    return Ok(Some(TranscriptBundle {
                        video_id: probe.video_id.clone(),
                        source: "subtitle_manual".into(),
                        language: "local".into(),
                        segments: segs,
                    }));
                }
            }
        }
        if probe
            .manual_sub_langs
            .iter()
            .any(|s| s.starts_with("embedded:"))
            && let Some(ffmpeg) = toolchain.ffmpeg_path.as_deref()
        {
            let out_vtt = subs_dir.join("embedded.vtt");
            let out_str = out_vtt.to_string_lossy().into_owned();
            if run_cmd(
                ffmpeg,
                &["-y", "-i", vpath_str, "-map", "0:s:0", &out_str],
                &[],
            )
            .await
            .is_ok()
                && out_vtt.is_file()
            {
                let segs = parse_subtitle_file(&out_vtt)?;
                if !segs.is_empty() {
                    return Ok(Some(TranscriptBundle {
                        video_id: probe.video_id.clone(),
                        source: "subtitle_manual".into(),
                        language: "embedded".into(),
                        segments: segs,
                    }));
                }
            }
        }
        return Ok(None);
    }

    let Some(source_url) = probe.source_url.as_deref() else {
        return Ok(None);
    };
    let Some(yt_dlp) = toolchain.yt_dlp_path.as_deref() else {
        return Ok(None);
    };

    let (chosen_lang, is_manual) = if let Some(lang) = pick_best_sub_lang(&probe.manual_sub_langs) {
        (lang, true)
    } else if policy == TranscriptPolicy::PreferAnySubtitles
        && let Some(lang) = pick_best_sub_lang(&probe.auto_sub_langs)
    {
        (lang, false)
    } else {
        return Ok(None);
    };

    let out_tpl = subs_dir.join("sub.%(ext)s").to_string_lossy().into_owned();
    let mut args = vec![
        "--skip-download",
        "--no-warnings",
        "--no-playlist",
        "--sub-langs",
        &chosen_lang,
        "--sub-format",
        "json3/vtt/srt/best",
        "-o",
        &out_tpl,
    ];
    args.push(if is_manual {
        "--write-subs"
    } else {
        "--write-auto-subs"
    });
    if let Some(browser) = cookies_from_browser.filter(|s| !s.trim().is_empty()) {
        args.push("--cookies-from-browser");
        args.push(browser);
    }
    args.push(source_url);

    if run_cmd(yt_dlp, &args, &[]).await.is_err() {
        return Ok(None);
    }

    let mut found: Vec<PathBuf> = std::fs::read_dir(&subs_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
                matches!(ext.to_ascii_lowercase().as_str(), "json3" | "vtt" | "srt")
            })
        })
        .collect();
    found.sort_by_key(
        |p| match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "json3" => 0,
            "vtt" => 1,
            _ => 2,
        },
    );

    let Some(sub_file) = found.first() else {
        return Ok(None);
    };
    let segs = parse_subtitle_file(sub_file)?;
    if segs.is_empty() {
        return Ok(None);
    }

    Ok(Some(TranscriptBundle {
        video_id: probe.video_id.clone(),
        source: if is_manual {
            "subtitle_manual".into()
        } else {
            "subtitle_auto".into()
        },
        language: chosen_lang,
        segments: segs,
    }))
}

pub fn parse_subtitle_file(path: &Path) -> Result<Vec<TranscriptSegment>, VideoError> {
    let content = std::fs::read_to_string(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let raw = if ext == "json3" {
        parse_youtube_json3(&content)
    } else {
        parse_vtt_or_srt(&content)
    };
    Ok(merge_micro_segments(raw))
}

pub fn parse_youtube_json3(content: &str) -> Vec<TranscriptSegment> {
    let Ok(data) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let Some(events) = data["events"].as_array() else {
        return Vec::new();
    };
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    for ev in events {
        let Some(segs) = ev["segs"].as_array() else {
            continue;
        };
        let start_ms = ev["tStartMs"].as_u64().unwrap_or(0);
        let dur_ms = ev["dDurationMs"].as_u64().unwrap_or(2000);
        let mut pieces = String::new();
        for s in segs {
            if let Some(u) = s["utf8"].as_str()
                && u != "\n"
            {
                pieces.push_str(u);
            }
        }
        let text = clean_caption_line(&pieces);
        if text.is_empty() {
            continue;
        }
        if let Some(last) = segments.last_mut()
            && is_rolling_duplicate(&last.text, &text)
        {
            if text.len() > last.text.len() {
                last.text = text;
                last.end_ms = last.end_ms.max(start_ms + dur_ms);
            }
            continue;
        }
        segments.push(TranscriptSegment {
            index: segments.len(),
            start_ms,
            end_ms: start_ms + dur_ms,
            text,
        });
    }
    clamp_overlaps(&mut segments);
    segments
}

pub fn parse_vtt_or_srt(content: &str) -> Vec<TranscriptSegment> {
    let lines: Vec<&str> = content.lines().collect();
    let mut cues: Vec<(u64, u64, Vec<String>)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(cap) = VTT_TIMING_RE.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let start_ms = parse_timestamp_to_ms(&cap[1]);
        let end_ms = parse_timestamp_to_ms(&cap[2]).max(start_ms + 200);
        i += 1;
        let mut cue_lines = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            let cleaned = clean_caption_line(lines[i]);
            if !cleaned.is_empty() {
                cue_lines.push(cleaned);
            }
            i += 1;
        }
        if !cue_lines.is_empty() {
            cues.push((start_ms, end_ms, cue_lines));
        }
        i += 1;
    }

    let mut segments = Vec::new();
    let mut recent: Vec<String> = Vec::new();
    for (start_ms, end_ms, cue_lines) in cues {
        let mut fresh = Vec::new();
        for ln in cue_lines {
            let dup = recent
                .iter()
                .rev()
                .take(3)
                .any(|prev| is_rolling_duplicate(prev, &ln));
            if !dup {
                fresh.push(ln.clone());
                recent.push(ln);
            }
        }
        let joined = fresh.join(" ");
        let joined = joined.trim();
        if !joined.is_empty() {
            segments.push(TranscriptSegment {
                index: segments.len(),
                start_ms,
                end_ms,
                text: joined.to_string(),
            });
        }
    }
    clamp_overlaps(&mut segments);
    segments
}

fn clamp_overlaps(segments: &mut [TranscriptSegment]) {
    for i in 0..segments.len().saturating_sub(1) {
        if segments[i].end_ms > segments[i + 1].start_ms {
            segments[i].end_ms = segments[i + 1].start_ms;
        }
    }
}

pub fn merge_micro_segments(segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let Some(mut buf) = segments.first().cloned() else {
        return Vec::new();
    };
    let mut merged = Vec::new();
    for seg in segments.into_iter().skip(1) {
        let gap = seg.start_ms.saturating_sub(buf.end_ms);
        let ends_sentence = buf
            .text
            .ends_with(['.', '!', '?', '。', '！', '？', ';', '；']);
        let combined_chars = buf.text.chars().count() + 1 + seg.text.chars().count();
        let buf_chars = buf.text.chars().count();
        if gap <= 1500 && combined_chars <= 160 && (buf_chars < 28 || !ends_sentence) {
            let sep = if is_cjk_boundary(&buf.text, &seg.text) {
                ""
            } else {
                " "
            };
            buf.text = format!("{}{sep}{}", buf.text, seg.text);
            buf.end_ms = buf.end_ms.max(seg.end_ms);
        } else {
            buf.index = merged.len();
            merged.push(buf);
            buf = seg;
        }
    }
    buf.index = merged.len();
    merged.push(buf);
    merged
}

fn clean_caption_line(raw: &str) -> String {
    let no_tags = INLINE_TAG_RE.replace_all(raw, "");
    let unescaped = no_tags
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    let collapsed = unescaped.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(
        collapsed.as_str(),
        "[Music]" | "[Applause]" | "[音乐]" | "[掌声]"
    ) {
        String::new()
    } else {
        collapsed
    }
}

fn is_rolling_duplicate(prev: &str, cur: &str) -> bool {
    let p = prev.trim().to_lowercase();
    let c = cur.trim().to_lowercase();
    if p.is_empty() || c.is_empty() {
        return false;
    }
    p == c || (c.starts_with(&p) && c.chars().count().saturating_sub(p.chars().count()) <= 3)
}

fn is_cjk_boundary(left: &str, right: &str) -> bool {
    let Some(lc) = left.chars().last() else {
        return false;
    };
    let Some(rc) = right.chars().next() else {
        return false;
    };
    ('\u{4e00}'..='\u{9fff}').contains(&lc) && ('\u{4e00}'..='\u{9fff}').contains(&rc)
}

/// Ensures a local MP4 video exists for frame extraction (downloads remote URLs up to 1080p).
pub async fn ensure_local_video(
    probe: &VideoProbe,
    work_dir: &Path,
    toolchain: &ToolchainStatus,
    cookies_from_browser: Option<&str>,
) -> Result<PathBuf, VideoError> {
    if let Some(local) = &probe.local_video_path {
        let p = PathBuf::from(local);
        if p.is_file() {
            return Ok(p);
        }
    }
    let cached = work_dir.join("video.mp4");
    if cached.is_file() && cached.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(cached);
    }
    let source_url = probe
        .source_url
        .as_deref()
        .ok_or_else(|| VideoError::Media("No source URL or local video file available.".into()))?;
    let yt_dlp = toolchain.yt_dlp_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain("`yt-dlp` is required to download online videos.".into())
    })?;

    std::fs::create_dir_all(work_dir)?;
    let out_str = cached.to_string_lossy().into_owned();
    let fmt = "bestvideo[height<=1080][ext=mp4]+bestaudio[ext=m4a]/bestvideo[height<=1080]+bestaudio/best[height<=1080]/best";
    let mut args = vec![
        "--no-warnings",
        "--no-playlist",
        "-f",
        fmt,
        "--merge-output-format",
        "mp4",
        "-o",
        &out_str,
    ];
    if let Some(browser) = cookies_from_browser.filter(|s| !s.trim().is_empty()) {
        args.push("--cookies-from-browser");
        args.push(browser);
    }
    args.push(source_url);
    if let Err(e) = run_cmd(yt_dlp, &args, &[]).await {
        if cookies_from_browser.is_some() {
            run_cmd(
                yt_dlp,
                &[
                    "--no-warnings",
                    "--no-playlist",
                    "-f",
                    fmt,
                    "--merge-output-format",
                    "mp4",
                    "-o",
                    &out_str,
                    source_url,
                ],
                &[],
            )
            .await?;
        } else {
            return Err(e);
        }
    }

    if !cached.is_file() {
        for entry in std::fs::read_dir(work_dir)?.flatten() {
            let p = entry.path();
            if p.file_stem().and_then(|s| s.to_str()) == Some("video") && p.is_file() {
                std::fs::rename(&p, &cached)?;
                break;
            }
        }
    }
    if !cached.is_file() {
        return Err(VideoError::Media(
            "Video download completed without producing video.mp4.".into(),
        ));
    }
    Ok(cached)
}

/// Extracts a compact 16kHz mono MP3 stream for ASR.
pub async fn ensure_local_audio(
    probe: &VideoProbe,
    local_video: Option<&Path>,
    work_dir: &Path,
    toolchain: &ToolchainStatus,
    cookies_from_browser: Option<&str>,
) -> Result<PathBuf, VideoError> {
    let cached = work_dir.join("audio_16k.mp3");
    if cached.is_file() && cached.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(cached);
    }
    let ffmpeg = toolchain.ffmpeg_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain("`ffmpeg` is required for audio extraction.".into())
    })?;
    std::fs::create_dir_all(work_dir)?;

    let input_path: PathBuf = if let Some(v) = local_video.filter(|p| p.is_file()) {
        v.to_path_buf()
    } else if let Some(local) = probe
        .local_video_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        local
    } else if let Some(url) = probe.source_url.as_deref() {
        let yt_dlp = toolchain.yt_dlp_path.as_deref().ok_or_else(|| {
            VideoError::Toolchain("`yt-dlp` is required to fetch online audio.".into())
        })?;
        let tpl = work_dir
            .join("raw_audio.%(ext)s")
            .to_string_lossy()
            .into_owned();
        let mut args = vec![
            "--no-warnings",
            "--no-playlist",
            "-f",
            "bestaudio/best",
            "-o",
            &tpl,
        ];
        if let Some(browser) = cookies_from_browser.filter(|s| !s.trim().is_empty()) {
            args.push("--cookies-from-browser");
            args.push(browser);
        }
        args.push(url);
        if let Err(e) = run_cmd(yt_dlp, &args, &[]).await {
            if cookies_from_browser.is_some() {
                run_cmd(
                    yt_dlp,
                    &[
                        "--no-warnings",
                        "--no-playlist",
                        "-f",
                        "bestaudio/best",
                        "-o",
                        &tpl,
                        url,
                    ],
                    &[],
                )
                .await?;
            } else {
                return Err(e);
            }
        }
        std::fs::read_dir(work_dir)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some("raw_audio"))
            .ok_or_else(|| VideoError::Media("Failed to download audio stream.".into()))?
    } else {
        return Err(VideoError::Media(
            "No media source available for audio extraction.".into(),
        ));
    };

    let in_str = input_path.to_string_lossy().into_owned();
    let out_str = cached.to_string_lossy().into_owned();
    run_cmd(
        ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
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
            "64k",
            &out_str,
        ],
        &[],
    )
    .await?;
    Ok(cached)
}

/// Builds a compact 640p 0.5fps + 16kbps mono MP4 proxy for Gemini inline delivery.
pub async fn build_compact_proxy_video(
    ffmpeg: &str,
    source_video: &Path,
    out_proxy: &Path,
    start_ms: u64,
    duration_ms: Option<u64>,
) -> Result<(), VideoError> {
    if out_proxy.is_file() && out_proxy.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    if let Some(parent) = out_proxy.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let start_s = format!("{:.3}", start_ms as f64 / 1000.0);
    let dur_s = duration_ms.map(|d| format!("{:.3}", d as f64 / 1000.0));
    let in_str = source_video.to_string_lossy().into_owned();
    let out_str = out_proxy.to_string_lossy().into_owned();

    let mut args = vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-hwaccel",
        "auto",
    ];
    if start_ms > 0 {
        args.extend(["-ss", &start_s]);
    }
    if let Some(ref ds) = dur_s {
        args.extend(["-t", ds]);
    }
    args.extend([
        "-i",
        &in_str,
        "-vf",
        "fps=0.5,scale='min(640,iw)':-2",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "34",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-c:a",
        "aac",
        "-b:a",
        "16k",
        &out_str,
    ]);
    run_cmd(ffmpeg, &args, &[]).await?;
    Ok(())
}

#[derive(Clone, Debug)]
struct FrameMetrics {
    dhash: u64,
    sharpness: f64,
    luma_std: f64,
}

/// Computes 64-bit dHash, Laplacian variance sharpness, and luminance std by decoding a 320x180
/// raw grayscale buffer via `ffmpeg` (no external image codec crate required).
async fn analyze_frame_via_ffmpeg(ffmpeg: &str, image_path: &Path) -> Option<FrameMetrics> {
    let in_str = image_path.to_string_lossy().into_owned();
    let output = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &in_str,
            "-vf",
            "scale=320:180,format=gray",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    let buf = output.stdout;
    const W: usize = 320;
    const H: usize = 180;
    if !output.status.success() || buf.len() < W * H {
        return None;
    }

    // 1. Luminance mean & std
    let n = (W * H) as f64;
    let sum: f64 = buf[..W * H].iter().map(|&b| b as f64).sum();
    let mean = sum / n;
    let var: f64 = buf[..W * H]
        .iter()
        .map(|&b| {
            let d = b as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    let luma_std = var.sqrt();

    // 2. Discrete Laplacian variance
    let mut lap_vals = Vec::with_capacity((W - 2) * (H - 2));
    for y in 1..(H - 1) {
        for x in 1..(W - 1) {
            let c = buf[y * W + x] as f64;
            let l = buf[(y - 1) * W + x] as f64
                + buf[(y + 1) * W + x] as f64
                + buf[y * W + (x - 1)] as f64
                + buf[y * W + (x + 1)] as f64
                - 4.0 * c;
            lap_vals.push(l);
        }
    }
    let lap_mean = lap_vals.iter().sum::<f64>() / lap_vals.len() as f64;
    let sharpness = lap_vals
        .iter()
        .map(|v| {
            let d = *v - lap_mean;
            d * d
        })
        .sum::<f64>()
        / lap_vals.len() as f64;

    // 3. 9x8 dHash downsampled from 320x180
    let mut small = [[0.0f64; 9]; 8];
    for (gy, row) in small.iter_mut().enumerate() {
        let y0 = gy * H / 8;
        let y1 = ((gy + 1) * H / 8).max(y0 + 1);
        for (gx, cell) in row.iter_mut().enumerate() {
            let x0 = gx * W / 9;
            let x1 = ((gx + 1) * W / 9).max(x0 + 1);
            let mut acc = 0.0;
            let mut cnt = 0usize;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    acc += buf[yy * W + xx] as f64;
                    cnt += 1;
                }
            }
            *cell = acc / cnt.max(1) as f64;
        }
    }
    let mut dhash = 0u64;
    let mut bit = 0;
    for row in &small {
        for x in 0..8 {
            if row[x] < row[x + 1] {
                dhash |= 1u64 << bit;
            }
            bit += 1;
        }
    }

    Some(FrameMetrics {
        dhash,
        sharpness,
        luma_std,
    })
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub async fn extract_single_frame(
    ffmpeg: &str,
    video_path: &Path,
    timestamp_ms: u64,
    out_path: &Path,
    max_width: u32,
) -> bool {
    let sec = format!("{:.3}", timestamp_ms as f64 / 1000.0);
    let vf = format!("scale='min({max_width},iw)':-2");
    let in_str = video_path.to_string_lossy().into_owned();
    let out_str = out_path.to_string_lossy().into_owned();
    run_cmd(
        ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-hwaccel",
            "auto",
            "-ss",
            &sec,
            "-i",
            &in_str,
            "-vf",
            &vf,
            "-frames:v",
            "1",
            "-q:v",
            "3",
            &out_str,
        ],
        &[],
    )
    .await
    .is_ok()
        && out_path.is_file()
}

/// Extracts sharp Retina (1920px) frames at requested timestamps, probing `[0ms, +650ms, -500ms]`.
pub async fn extract_frames_at_timestamps(
    ffmpeg: &str,
    video_path: &Path,
    timestamps_ms: &[u64],
    out_dir: &Path,
    prefix: &str,
) -> Result<Vec<CandidateFrame>, VideoError> {
    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    let offsets: [i64; 3] = [0, 650, -500];

    for (idx, &ts_ms) in timestamps_ms.iter().enumerate() {
        let fid = format!("{prefix}{:03}", idx + 1);
        let final_file = out_dir.join(format!("{fid}_{ts_ms:08}.jpg"));

        let mut best: Option<(PathBuf, u64, FrameMetrics)> = None;
        let mut probes = Vec::new();
        for (p_idx, &off) in offsets.iter().enumerate() {
            let cand_ts = (ts_ms as i64 + off).max(200) as u64;
            let p_file = out_dir.join(format!("{fid}_{ts_ms:08}_p{p_idx}.jpg"));
            probes.push(p_file.clone());
            if extract_single_frame(ffmpeg, video_path, cand_ts, &p_file, 1920).await
                && let Some(m) = analyze_frame_via_ffmpeg(ffmpeg, &p_file).await
                && m.luma_std >= 6.0
            {
                let replace = match &best {
                    None => true,
                    Some((_, _, prev_m)) => m.sharpness > prev_m.sharpness * 1.12,
                };
                if replace {
                    best = Some((p_file, cand_ts, m));
                }
            }
        }

        if let Some((chosen_path, chosen_ts, m)) = best {
            let _ = std::fs::remove_file(&final_file);
            std::fs::rename(&chosen_path, &final_file)?;
            out.push(CandidateFrame {
                frame_id: fid,
                timestamp_ms: chosen_ts,
                image_path: final_file.to_string_lossy().into_owned(),
                thumb_path: None,
                dhash: m.dhash,
                sharpness: m.sharpness,
            });
        }
        for p in probes {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(out)
}

/// Fallback local candidate frame extraction (scene detection 0.025 + progressive-slide tail-frame selection + dHash dedup).
pub async fn build_local_candidate_frames(
    ffmpeg: &str,
    video_path: &Path,
    probe: &VideoProbe,
    transcript: &TranscriptBundle,
    work_dir: &Path,
    max_candidates: usize,
) -> Result<Vec<CandidateFrame>, VideoError> {
    let frames_dir = work_dir.join("local_frames");
    let raw_dir = frames_dir.join("raw");
    let keep_dir = frames_dir.join("candidates");
    let thumb_dir = frames_dir.join("thumbs");
    for d in [&raw_dir, &keep_dir, &thumb_dir] {
        std::fs::create_dir_all(d)?;
    }

    let mut scene_ms = detect_scene_changes_ms(ffmpeg, video_path, 0.025).await;
    if scene_ms.len() < 2 {
        scene_ms = detect_scene_changes_ms(ffmpeg, video_path, 0.01).await;
    }

    let duration_ms = probe.duration_ms.max(10_000);
    let upper = duration_ms.saturating_sub(300).max(500);
    let mut pts: Vec<u64> = Vec::new();
    let mut add_pt = |ms: u64| {
        let clamped = ms.clamp(200, upper);
        if pts
            .iter()
            .all(|&existing| existing.abs_diff(clamped) >= 600)
        {
            pts.push(clamped);
        }
    };

    add_pt(1200.min(upper / 2));
    for ch in &probe.chapters {
        add_pt(ch.start_ms + 800);
        if ch.end_ms > ch.start_ms + 2000 {
            add_pt(ch.end_ms.saturating_sub(350));
        }
    }
    for &st in &scene_ms {
        if st > 850 {
            add_pt(st - 350);
        }
        add_pt(st + 450);
    }
    if duration_ms > 3000 {
        add_pt(duration_ms - 1200);
    }
    pts.sort_unstable();

    // Fill >20s gaps using transcript anchors
    let mut filled = pts.clone();
    let mut bounds = vec![0u64];
    bounds.extend(&pts);
    bounds.push(duration_ms);
    for win in bounds.windows(2) {
        let (l, r) = (win[0], win[1]);
        let gap = r.saturating_sub(l);
        if gap > 20_000 {
            let steps = ((gap as f64 / 20_000.0).ceil() as u64)
                .saturating_sub(1)
                .max(1);
            for s in 1..=steps {
                let target = l + gap * s / (steps + 1);
                let snapped = transcript
                    .segments
                    .iter()
                    .map(|seg| seg.end_ms.saturating_sub(200).max(seg.start_ms + 200))
                    .min_by_key(|a| a.abs_diff(target))
                    .filter(|a| a.abs_diff(target) <= 4000)
                    .unwrap_or(target);
                if filled.iter().all(|&e| e.abs_diff(snapped) >= 600) {
                    filled.push(snapped.clamp(200, upper));
                }
            }
        }
    }
    filled.sort_unstable();

    let mut samples: Vec<(u64, PathBuf, FrameMetrics)> = Vec::new();
    for (idx, &ts) in filled.iter().enumerate() {
        let raw_file = raw_dir.join(format!("raw_{idx:04}_{ts:08}.jpg"));
        if extract_single_frame(ffmpeg, video_path, ts, &raw_file, 1920).await
            && let Some(m) = analyze_frame_via_ffmpeg(ffmpeg, &raw_file).await
            && m.luma_std >= 8.0
            && m.sharpness >= 12.0
        {
            samples.push((ts, raw_file, m));
        }
    }

    // Cluster consecutive similar slides (dHash <= 9) and pick the last sharp frame in each cluster
    let mut clusters: Vec<Vec<(u64, PathBuf, FrameMetrics)>> = Vec::new();
    for s in samples {
        if let Some(last_cluster) = clusters.last_mut()
            && let Some(prev) = last_cluster.last()
            && hamming_distance(prev.2.dhash, s.2.dhash) <= 9
        {
            last_cluster.push(s);
            continue;
        }
        clusters.push(vec![s]);
    }

    let mut winners: Vec<(u64, PathBuf, FrameMetrics)> = Vec::new();
    for cluster in clusters {
        let max_sharp = cluster.iter().map(|x| x.2.sharpness).fold(0.0f64, f64::max);
        let chosen = cluster
            .iter()
            .rev()
            .find(|x| x.2.sharpness >= max_sharp * 0.78)
            .or_else(|| cluster.last())
            .cloned();
        if let Some(w) = chosen {
            let dup_recent = winners.iter().rev().take(4).any(|p| {
                w.0.saturating_sub(p.0) < 90_000 && hamming_distance(w.2.dhash, p.2.dhash) <= 6
            });
            if !dup_recent {
                winners.push(w);
            }
        }
    }

    if winners.len() > max_candidates {
        let step = winners.len() as f64 / max_candidates as f64;
        winners = (0..max_candidates)
            .map(|i| {
                let idx = ((i as f64 * step).round() as usize).min(winners.len() - 1);
                winners[idx].clone()
            })
            .collect();
    }

    let mut candidates = Vec::new();
    for (i, (ts, src_path, m)) in winners.into_iter().enumerate() {
        let fid = format!("f{:03}", i + 1);
        let hd_dest = keep_dir.join(format!("{fid}_{ts:08}.jpg"));
        let thumb_dest = thumb_dir.join(format!("{fid}.jpg"));
        std::fs::copy(&src_path, &hd_dest)?;
        let hd_str = hd_dest.to_string_lossy().into_owned();
        let th_str = thumb_dest.to_string_lossy().into_owned();
        let _ = run_cmd(
            ffmpeg,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                &hd_str,
                "-vf",
                "scale=640:-2",
                "-q:v",
                "5",
                &th_str,
            ],
            &[],
        )
        .await;

        candidates.push(CandidateFrame {
            frame_id: fid,
            timestamp_ms: ts,
            image_path: hd_str,
            thumb_path: thumb_dest.is_file().then_some(th_str),
            dhash: m.dhash,
            sharpness: m.sharpness,
        });
    }
    let _ = std::fs::remove_dir_all(&raw_dir);
    Ok(candidates)
}

async fn detect_scene_changes_ms(ffmpeg: &str, video_path: &Path, threshold: f64) -> Vec<u64> {
    let in_str = video_path.to_string_lossy().into_owned();
    let vf = format!("scale=640:-2,select='gt(scene,{threshold:.3})',showinfo");
    let Ok(output) = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-nostats",
            "-hwaccel",
            "auto",
            "-i",
            &in_str,
            "-vf",
            &vf,
            "-an",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
    else {
        return Vec::new();
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut times: Vec<u64> = PTS_TIME_RE
        .captures_iter(&stderr)
        .filter_map(|c| c[1].parse::<f64>().ok())
        .map(|s| (s * 1000.0).round() as u64)
        .collect();
    times.sort_unstable();
    times.dedup();
    times
}
