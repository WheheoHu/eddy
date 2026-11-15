use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::VideoError;
use crate::asr::transcribe_with_api;
use crate::client::{
    UserPart, build_streaming_http_client, call_gemini_generate_content,
    call_gemini_generate_content_with_retries, call_stage_model, extract_json_object,
};
use crate::epub_builder::build_epub_bytes;
use crate::media::{
    build_compact_proxy_video, build_local_candidate_frames, ensure_local_audio,
    ensure_local_video, extract_frames_at_timestamps, hamming_distance, probe_video,
    try_fetch_subtitles,
};
use crate::models::{
    ArticleBlock, ArticleDocument, ArticleSection, CandidateFrame, GeneratedBook, PipelinePhase,
    PipelineProgress, Stage1ChapterPlan, Stage1Plan, Stage1SlideChoice, TranscriptBundle,
    VideoProbe, format_timestamp_ms, parse_timestamp_to_ms,
};
use crate::settings::{RuntimeVideoConfig, StageEndpoint, VideoPipelineMode};
use crate::toolchain::detect_toolchain;

pub type ProgressCallback = Arc<dyn Fn(PipelineProgress) + Send + Sync>;

const MAX_INLINE_CHUNK_MS: u64 = 20 * 60 * 1000;
const TARGET_INLINE_CHUNK_MS: u64 = 16 * 60 * 1000;

const GEMINI_STAGE1_PROMPT: &str = r#"You are an expert technical editor performing Stage 1 (Visual Slide Localization & Chapter Planning) on a presentation video.
A specialized deep-writing LLM will write each chapter in Stage 2 using your plan and the exact HD video frames captured at your timestamps.

{clip_context}

Instructions:
1. Segment this video ({clip_span_note}) into {chapter_count_hint} logical, coherent chapters (`start_time` and `end_time` in `MM:SS` relative to THIS video clip starting at `00:00`).
   - Include slide-less spoken sections too (such as live Q&A or executive transitions) with `"slides": []` so no spoken content is lost.
2. Watch the visual stream carefully and pinpoint high-value visual moments worth embedding in a written report (aim for 3 to 8 distinct visuals per chapter):
   - PRIORITIZE: presentation slides, Bento summary cards, architecture diagrams, spec/benchmark charts, UI feature callouts, code/profiler views, and key demo reveal frames.
   - EXCLUDE: pure speaker talking-head shots, decorative B-roll camera-angle repeats, crowd/stage wide shots, or blurry fade transitions.
   - PROGRESSIVE SLIDES RULE: When bullet points, Bento tiles, or diagram layers appear incrementally, pick the timestamp (`MM:SS` relative to `00:00` of this clip) when ALL items on that slide have appeared, ~1 second BEFORE it transitions away!
3. Keep `draft_caption` in {target_language} concise and factual (including exact English product/technical terms, specs, or numbers visible on screen).

Return STRICT JSON matching this exact schema:
{
  "article_title": "Refined publication-grade title in {target_language}",
  "subtitle": "One-sentence framing subtitle in {target_language}",
  "executive_summary": "3-5 sentence synthesis of the core thesis and announcements in {target_language}",
  "key_takeaways": ["Takeaway 1", "Takeaway 2", "Takeaway 3", "Takeaway 4"],
  "chapters": [
    {
      "chapter_index": 1,
      "title": "Chapter title in {target_language}",
      "start_time": "00:00",
      "end_time": "04:15",
      "summary_goal": "Specific technical details, specs, and arguments Stage 2 must explain in depth",
      "key_terms": ["TermA", "TermB"],
      "slides": [
        {
          "timestamp": "02:33",
          "role": "diagram|slide|bento_summary|demo|chart",
          "draft_caption": "Specific caption describing what this visual shows"
        }
      ]
    }
  ]
}
"#;

const LOCAL_FRAMES_STAGE1_SYSTEM: &str = r#"You are an expert technical editor preparing a structured visual article plan from a recorded presentation.
1. Segment the presentation into 4 to 9 coherent chapters (`start_time` and `end_time` in `MM:SS`), including slide-less Q&A chapters if present.
2. Inspect the attached candidate frames (chronological order matching the Frame Catalog) and KEEP only informative slides, diagrams, code/UI demos, and charts; DROP pure speaker talking-head shots and blurry transitions.
3. Assign each kept `frame_id` to the chapter where it is explained.

Return STRICT JSON:
{
  "article_title": "Refined publication-grade title",
  "subtitle": "One-sentence framing subtitle",
  "executive_summary": "3-5 sentence synthesis",
  "key_takeaways": ["Takeaway 1", "Takeaway 2"],
  "chapters": [
    {
      "chapter_index": 1,
      "title": "Chapter title",
      "start_time": "00:00",
      "end_time": "04:15",
      "summary_goal": "What this chapter covers in depth",
      "key_terms": ["TermA"],
      "selected_slides": [
        {"frame_id": "f002", "role": "slide", "draft_caption": "Concise caption"}
      ]
    }
  ]
}
"#;

const STAGE2_WRITER_SYSTEM: &str = r#"You are an authoritative technical writer turning one chapter of a spoken presentation into a polished, reflowable long-form book chapter.

Writing rules:
1. Write in clear, natural {target_language}. Preserve technical identifiers, API names, model/chip names, and code in their original exact spelling.
2. DO NOT summarize into shallow bullet points! Preserve the speaker's concrete arguments, numbers, specs, trade-offs, examples, and reasoning depth, while completely removing spoken filler ("uh", "you know", "as you see on this slide").
3. Resolve deictic visual references ("look at this box on the top right") into explicit self-contained prose paired with the embedded figure.
4. Embed EVERY assigned `frame_id` for this chapter as `{"type": "figure", "frame_id": "hs001", "caption": "..."}` right beside the paragraph explaining it. Do not invent non-existent `frame_id`s.
5. When a slide or transcript presents code, mathematical formulas, or structured comparisons/specs, transcribe them cleanly as `code`, `math` (LaTeX without `$$` delimiters), or `table` blocks.

Return STRICT JSON with this schema:
{
  "title": "Polished chapter heading",
  "blocks": [
    {"type": "paragraph", "text": "..."},
    {"type": "figure", "frame_id": "hs001", "caption": "..."},
    {"type": "subheading", "text": "..."},
    {"type": "bullet_list", "items": ["...", "..."]},
    {"type": "code", "language": "rust", "text": "..."},
    {"type": "math", "tex": "L = \\sum_{i=1}^N x_i"},
    {"type": "table", "headers": ["Col A", "Col B"], "rows": [["1", "2"]]},
    {"type": "quote", "text": "Memorable insight from the talk"}
  ]
}
"#;

#[derive(Serialize, Deserialize)]
struct PlanCheckpoint {
    plan: Stage1Plan,
    frames: Vec<CandidateFrame>,
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), VideoError> {
    if cancel.load(Ordering::Relaxed) {
        Err(VideoError::Canceled)
    } else {
        Ok(())
    }
}

/// Executes or resumes the full Video-to-Book workflow inside `job_dir`, saving checkpoints after
/// every stage so retries skip already-completed ASR, visual planning, frame capture, and chapters.
pub async fn run_video_pipeline(
    source: &str,
    job_dir: &Path,
    cfg: &RuntimeVideoConfig,
    cancel: Arc<AtomicBool>,
    on_progress: Option<ProgressCallback>,
) -> Result<GeneratedBook, VideoError> {
    std::fs::create_dir_all(job_dir)?;
    let report =
        |phase: PipelinePhase, done: u32, total: u32, detail: &str, title: Option<String>| {
            if let Some(cb) = &on_progress {
                cb(PipelineProgress {
                    phase,
                    completed_chapters: done,
                    total_chapters: total,
                    detail: detail.to_string(),
                    resolved_title: title,
                });
            }
        };

    check_cancel(&cancel)?;
    let toolchain = detect_toolchain(cfg.ffmpeg_path.as_deref(), cfg.yt_dlp_path.as_deref());
    let ffmpeg = toolchain.ffmpeg_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain("`ffmpeg` was not found. Install it via `brew install ffmpeg` or set its path in Settings -> Video.".into())
    })?;
    let ffprobe = toolchain.ffprobe_path.as_deref().ok_or_else(|| {
        VideoError::Toolchain("`ffprobe` was not found alongside `ffmpeg`.".into())
    })?;

    // --- Step 1: Probe metadata (checkpointed) ---
    report(
        PipelinePhase::Probing,
        0,
        0,
        "Inspecting video metadata…",
        None,
    );
    let probe_file = job_dir.join("checkpoint_probe.json");
    let probe: VideoProbe = if probe_file.is_file() {
        serde_json::from_slice(&std::fs::read(&probe_file)?)
            .map_err(|e| VideoError::Format(e.to_string()))?
    } else {
        let p = probe_video(source, &toolchain, cfg.cookies_from_browser.as_deref()).await?;
        std::fs::write(
            &probe_file,
            serde_json::to_vec_pretty(&p).unwrap_or_default(),
        )?;
        p
    };
    report(
        PipelinePhase::Probing,
        0,
        0,
        &format!(
            "Loaded “{}” ({})",
            probe.title,
            format_timestamp_ms(probe.duration_ms)
        ),
        Some(probe.title.clone()),
    );

    check_cancel(&cancel)?;

    // --- Step 2: Obtain timestamped transcript (subtitles or VAD + API ASR, checkpointed) ---
    let transcript_file = job_dir.join("checkpoint_transcript.json");
    let transcript: TranscriptBundle = if transcript_file.is_file() {
        serde_json::from_slice(&std::fs::read(&transcript_file)?)
            .map_err(|e| VideoError::Format(e.to_string()))?
    } else {
        report(
            PipelinePhase::Transcribing,
            0,
            0,
            "Checking subtitles / transcribing speech…",
            Some(probe.title.clone()),
        );
        let sub = try_fetch_subtitles(
            &probe,
            job_dir,
            cfg.transcript_policy,
            &toolchain,
            cfg.cookies_from_browser.as_deref(),
        )
        .await?;

        let bundle = match sub {
            Some(b) => b,
            None => {
                let asr_ep = cfg.asr.as_ref().ok_or_else(|| {
                    VideoError::Config(
                        "This video has no usable subtitles; configure an ASR model (e.g. qwen3-asr-flash) in Settings -> Video."
                            .into(),
                    )
                })?;
                let local_v = probe.local_video_path.as_deref().map(Path::new);
                let audio_path = ensure_local_audio(
                    &probe,
                    local_v,
                    job_dir,
                    &toolchain,
                    cfg.cookies_from_browser.as_deref(),
                )
                .await?;
                let asr_prog_cb = on_progress.clone();
                let asr_title = probe.title.clone();
                let chunk_cb: Option<Arc<dyn Fn(usize, usize) + Send + Sync>> =
                    asr_prog_cb.map(|cb| {
                        Arc::new(move |done: usize, total: usize| {
                            cb(PipelineProgress {
                                phase: PipelinePhase::Transcribing,
                                completed_chapters: 0,
                                total_chapters: 0,
                                detail: format!(
                                    "Transcribing speech ({done} of {total} segments)…"
                                ),
                                resolved_title: Some(asr_title.clone()),
                            });
                        }) as Arc<dyn Fn(usize, usize) + Send + Sync>
                    });

                transcribe_with_api(
                    ffmpeg,
                    ffprobe,
                    &audio_path,
                    &probe.video_id,
                    asr_ep,
                    job_dir,
                    &probe.title,
                    chunk_cb,
                )
                .await?
            }
        };
        std::fs::write(
            &transcript_file,
            serde_json::to_vec_pretty(&bundle).unwrap_or_default(),
        )?;
        bundle
    };

    check_cancel(&cancel)?;

    // --- Step 3: Ensure local video + Stage 1 Visual Plan & HD Frame Capture (checkpointed) ---
    let plan_file = job_dir.join("checkpoint_plan.json");
    let (plan, frames): (Stage1Plan, Vec<CandidateFrame>) = if plan_file.is_file() {
        let cp: PlanCheckpoint = serde_json::from_slice(&std::fs::read(&plan_file)?)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        (cp.plan, cp.frames)
    } else {
        report(
            PipelinePhase::PlanningVisuals,
            0,
            0,
            "Preparing video stream and analyzing presentation slides…",
            Some(probe.title.clone()),
        );
        let local_video = ensure_local_video(
            &probe,
            job_dir,
            &toolchain,
            cfg.cookies_from_browser.as_deref(),
        )
        .await?;

        check_cancel(&cancel)?;

        let (p, f) = match cfg.effective_pipeline() {
            VideoPipelineMode::GeminiTwoStage => {
                run_gemini_stage1_and_capture(
                    ffmpeg,
                    &probe,
                    &local_video,
                    job_dir,
                    &cfg.stage1,
                    &cfg.target_language,
                    &report,
                )
                .await?
            }
            VideoPipelineMode::LocalFramesTwoStage | VideoPipelineMode::Auto => {
                run_local_frames_stage1(
                    ffmpeg,
                    &probe,
                    &transcript,
                    &local_video,
                    job_dir,
                    &cfg.vlm,
                    &cfg.target_language,
                    &report,
                )
                .await?
            }
        };

        let cp = PlanCheckpoint {
            plan: p.clone(),
            frames: f.clone(),
        };
        std::fs::write(
            &plan_file,
            serde_json::to_vec_pretty(&cp).unwrap_or_default(),
        )?;
        (p, f)
    };

    check_cancel(&cancel)?;

    // --- Step 4: Stage 2 Per-Chapter Deep Writing (checkpointed per chapter!) ---
    let total_chapters = plan.chapters.len() as u32;
    let resolved_book_title = if plan.article_title.trim().is_empty() {
        probe.title.clone()
    } else {
        plan.article_title.clone()
    };
    let chapters_dir = job_dir.join("chapters");
    std::fs::create_dir_all(&chapters_dir)?;

    let frame_lookup: HashMap<String, CandidateFrame> = frames
        .into_iter()
        .map(|c| (c.frame_id.clone(), c))
        .collect();

    let done_counter = Arc::new(AtomicU32::new(0));
    report(
        PipelinePhase::WritingChapters,
        0,
        total_chapters,
        &format!("Writing {total_chapters} chapters…"),
        Some(resolved_book_title.clone()),
    );

    let http = build_streaming_http_client(240, 1200)?;
    let sem = Arc::new(Semaphore::new(cfg.chapter_concurrency.clamp(1, 6)));
    let mut futs = FuturesUnordered::new();

    for ch in &plan.chapters {
        let ch_clone = ch.clone();
        let ch_file = chapters_dir.join(format!("chapter_{:02}.json", ch.chapter_index));
        let sem = sem.clone();
        let http = http.clone();
        let stage2_ep = cfg.stage2.clone();
        let target_lang = cfg.target_language.clone();
        let book_title = resolved_book_title.clone();
        let total_ch = total_chapters;
        let frame_map = frame_lookup.clone();
        let transcript_ref = &transcript;
        let cancel_ref = cancel.clone();
        let done_ref = done_counter.clone();
        let on_prog = on_progress.clone();

        let seg_text = transcript_ref
            .slice_ms(ch.start_ms, ch.end_ms, 2000)
            .into_iter()
            .map(|s| format!("[{}] {}", format_timestamp_ms(s.start_ms), s.text))
            .collect::<Vec<_>>()
            .join("\n");

        futs.push(async move {
            check_cancel(&cancel_ref)?;
            if ch_file.is_file()
                && let Ok(bytes) = std::fs::read(&ch_file)
                && let Ok(existing) = serde_json::from_slice::<ArticleSection>(&bytes)
            {
                let finished = done_ref.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(cb) = &on_prog {
                    cb(PipelineProgress {
                        phase: PipelinePhase::WritingChapters,
                        completed_chapters: finished,
                        total_chapters: total_ch,
                        detail: format!("Loaded cached chapter {finished} of {total_ch}"),
                        resolved_title: Some(book_title),
                    });
                }
                return Ok::<ArticleSection, VideoError>(existing);
            }

            let _permit = sem.acquire_owned().await.unwrap();
            check_cancel(&cancel_ref)?;

            let section = write_single_chapter(
                &http,
                &stage2_ep,
                &book_title,
                total_ch as usize,
                &ch_clone,
                &seg_text,
                &frame_map,
                &target_lang,
            )
            .await?;

            std::fs::write(
                &ch_file,
                serde_json::to_vec_pretty(&section).unwrap_or_default(),
            )?;
            let finished = done_ref.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(cb) = &on_prog {
                cb(PipelineProgress {
                    phase: PipelinePhase::WritingChapters,
                    completed_chapters: finished,
                    total_chapters: total_ch,
                    detail: format!("Wrote chapter {finished} of {total_ch}: {}", section.title),
                    resolved_title: Some(book_title),
                });
            }
            Ok(section)
        });
    }

    let mut sections = Vec::with_capacity(plan.chapters.len());
    while let Some(res) = futs.next().await {
        sections.push(res?);
    }
    sections.sort_by_key(|s| s.index);

    check_cancel(&cancel)?;

    // --- Step 5: Package Multi-Chapter EPUB 3 ---
    report(
        PipelinePhase::Packaging,
        total_chapters,
        total_chapters,
        "Packaging illustrated multi-chapter EPUB…",
        Some(resolved_book_title.clone()),
    );

    let article_doc = ArticleDocument {
        video_id: probe.video_id.clone(),
        title: resolved_book_title.clone(),
        subtitle: plan.subtitle,
        author: probe.author.clone(),
        source_url: probe.source_url.clone(),
        duration_ms: probe.duration_ms,
        executive_summary: plan.executive_summary,
        key_takeaways: plan.key_takeaways,
        sections,
    };

    let (epub_bytes, chapters_count) = build_epub_bytes(&article_doc)?;

    // Clean temporary large media streams while keeping lightweight JSON checkpoints
    let _ = std::fs::remove_file(job_dir.join("video.mp4"));
    let _ = std::fs::remove_file(job_dir.join("audio_16k.mp3"));
    if let Ok(entries) = std::fs::read_dir(job_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_stem().and_then(|s| s.to_str()) == Some("raw_audio") {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let _ = std::fs::remove_dir_all(job_dir.join("proxies"));

    report(
        PipelinePhase::Completed,
        total_chapters,
        total_chapters,
        "Completed",
        Some(resolved_book_title.clone()),
    );

    Ok(GeneratedBook {
        title: resolved_book_title,
        author: probe.author,
        chapters_count,
        epub_bytes,
    })
}

async fn run_gemini_stage1_and_capture<F>(
    ffmpeg: &str,
    probe: &VideoProbe,
    local_video: &Path,
    job_dir: &Path,
    stage1_ep: &StageEndpoint,
    target_language: &str,
    report: &F,
) -> Result<(Stage1Plan, Vec<CandidateFrame>), VideoError>
where
    F: Fn(PipelinePhase, u32, u32, &str, Option<String>),
{
    let http = build_streaming_http_client(300, 2400)?;
    let is_youtube = probe
        .source_url
        .as_deref()
        .is_some_and(|u| u.contains("youtube.com/watch") || u.contains("youtu.be/"));

    let mut raw_plan: Option<Value> = None;

    if is_youtube && let Some(url) = probe.source_url.as_deref() {
        report(
            PipelinePhase::PlanningVisuals,
            0,
            0,
            &format!(
                "Analyzing YouTube video stream with {}…",
                stage1_ep.model.trim()
            ),
            Some(probe.title.clone()),
        );
        let dur_label = format_timestamp_ms(probe.duration_ms);
        let prompt = GEMINI_STAGE1_PROMPT
            .replace("{target_language}", target_language)
            .replace(
                "{clip_context}",
                &format!("Presentation Title: {}", probe.title),
            )
            .replace("{clip_span_note}", &format!("00:00 to {dur_label}"))
            .replace("{chapter_count_hint}", "5 to 10");

        if let Ok(probe_http) = build_streaming_http_client(90, 240)
            && let Ok(out) = call_gemini_generate_content_with_retries(
                &probe_http,
                stage1_ep,
                "",
                &[
                    UserPart::FileUri {
                        mime: "video/mp4".into(),
                        uri: url.to_string(),
                    },
                    UserPart::Text(prompt),
                ],
                0.15,
                16384,
                true,
                0,
            )
            .await
            && let Ok(parsed) = extract_json_object(&out.text)
        {
            raw_plan = Some(parsed);
        }
    }

    let raw_plan = match raw_plan {
        Some(p) => p,
        None => {
            run_chunked_inline_gemini_stage1(
                &http,
                ffmpeg,
                probe,
                local_video,
                job_dir,
                stage1_ep,
                target_language,
                report,
            )
            .await?
        }
    };

    report(
        PipelinePhase::CapturingFrames,
        0,
        0,
        "Capturing high-resolution 1080p slides from video…",
        Some(probe.title.clone()),
    );
    let cap_dir = job_dir.join("gemini_captures");
    materialize_gemini_plan(ffmpeg, &raw_plan, probe, local_video, &cap_dir).await
}

#[allow(clippy::too_many_arguments)]
async fn run_chunked_inline_gemini_stage1<F>(
    http: &reqwest::Client,
    ffmpeg: &str,
    probe: &VideoProbe,
    local_video: &Path,
    job_dir: &Path,
    stage1_ep: &StageEndpoint,
    target_language: &str,
    report: &F,
) -> Result<Value, VideoError>
where
    F: Fn(PipelinePhase, u32, u32, &str, Option<String>),
{
    let total_ms = probe.duration_ms.max(60_000);
    let num_chunks = if total_ms <= MAX_INLINE_CHUNK_MS {
        1usize
    } else {
        ((total_ms as f64 / TARGET_INLINE_CHUNK_MS as f64).ceil() as usize).max(2)
    };
    let chunk_len_ms = total_ms.div_ceil(num_chunks as u64);
    let proxies_dir = job_dir.join("proxies");
    std::fs::create_dir_all(&proxies_dir)?;

    let mut windows: Vec<(usize, u64, u64, PathBuf)> = Vec::new();
    for idx in 0..num_chunks {
        let s_ms = idx as u64 * chunk_len_ms;
        let e_ms = ((idx as u64 + 1) * chunk_len_ms).min(total_ms);
        if e_ms > s_ms {
            let p = proxies_dir.join(format!("proxy_part_{:02}_of_{num_chunks:02}.mp4", idx + 1));
            windows.push((idx, s_ms, e_ms, p));
        }
    }

    report(
        PipelinePhase::PlanningVisuals,
        0,
        0,
        &format!("Encoding compact visual proxy ({} part(s))…", windows.len()),
        Some(probe.title.clone()),
    );
    for &(_, s_ms, e_ms, ref p_path) in &windows {
        let dur_opt = (num_chunks > 1).then_some(e_ms - s_ms);
        build_compact_proxy_video(ffmpeg, local_video, p_path, s_ms, dur_opt).await?;
    }

    report(
        PipelinePhase::PlanningVisuals,
        0,
        0,
        &format!(
            "Analyzing video stream with {} ({} part(s))…",
            stage1_ep.model.trim(),
            windows.len()
        ),
        Some(probe.title.clone()),
    );

    let mut parts = Vec::with_capacity(windows.len());
    for (idx, s_ms, e_ms, p_path) in windows {
        let part_cache =
            proxies_dir.join(format!("plan_part_{:02}_of_{num_chunks:02}.json", idx + 1));
        if part_cache.is_file()
            && let Ok(cached_bytes) = std::fs::read(&part_cache)
            && let Ok(cached_val) = serde_json::from_slice::<Value>(&cached_bytes)
            && cached_val.is_object()
        {
            parts.push((idx, s_ms, e_ms, cached_val));
            if num_chunks > 1 {
                report(
                    PipelinePhase::PlanningVisuals,
                    0,
                    0,
                    &format!(
                        "Loaded cached video plan ({} of {num_chunks} parts)…",
                        parts.len()
                    ),
                    Some(probe.title.clone()),
                );
            }
            continue;
        }

        if num_chunks > 1 {
            report(
                PipelinePhase::PlanningVisuals,
                0,
                0,
                &format!(
                    "Analyzing video stream with {} (part {} of {num_chunks})…",
                    stage1_ep.model.trim(),
                    idx + 1
                ),
                Some(probe.title.clone()),
            );
        }

        let clip_dur = e_ms - s_ms;
        let clip_label = format_timestamp_ms(clip_dur);
        let ctx = if num_chunks == 1 {
            format!("Presentation Title: {}", probe.title)
        } else {
            format!(
                "Presentation Title: {} (Part {} of {num_chunks}; clip length {clip_label}, starting at 00:00 of this clip)",
                probe.title,
                idx + 1
            )
        };
        let chap_hint = if num_chunks == 1 { "5 to 9" } else { "3 to 5" };
        let prompt = GEMINI_STAGE1_PROMPT
            .replace("{target_language}", target_language)
            .replace("{clip_context}", &ctx)
            .replace("{clip_span_note}", &format!("00:00 to {clip_label}"))
            .replace("{chapter_count_hint}", chap_hint);

        let bytes = std::fs::read(&p_path)?;
        let out = call_gemini_generate_content(
            http,
            stage1_ep,
            "",
            &[
                UserPart::InlineMedia {
                    mime: "video/mp4".into(),
                    bytes,
                },
                UserPart::Text(prompt),
            ],
            0.15,
            16384,
            true,
        )
        .await?;
        let parsed = extract_json_object(&out.text)?;
        let _ = std::fs::write(
            &part_cache,
            serde_json::to_vec_pretty(&parsed).unwrap_or_default(),
        );
        parts.push((idx, s_ms, e_ms, parsed));

        if num_chunks > 1 {
            report(
                PipelinePhase::PlanningVisuals,
                0,
                0,
                &format!(
                    "Analyzed video stream ({} of {num_chunks} parts)…",
                    parts.len()
                ),
                Some(probe.title.clone()),
            );
        }
    }
    parts.sort_by_key(|x| x.0);

    if parts.len() == 1 {
        return Ok(parts.into_iter().next().unwrap().3);
    }

    let mut merged_chapters = Vec::new();
    let mut summaries = Vec::new();
    let mut takeaways = Vec::new();
    let first_title = parts[0].3["article_title"]
        .as_str()
        .unwrap_or(&probe.title)
        .to_string();
    let first_sub = parts[0].3["subtitle"].as_str().unwrap_or("").to_string();

    for (_, s_ms, e_ms, part_val) in parts {
        let clip_dur = e_ms - s_ms;
        if let Some(sum) = part_val["executive_summary"].as_str()
            && !sum.trim().is_empty()
        {
            summaries.push(sum.trim().to_string());
        }
        if let Some(tks) = part_val["key_takeaways"].as_array() {
            for tk in tks {
                if let Some(s) = tk.as_str()
                    && !s.trim().is_empty()
                    && takeaways.len() < 8
                {
                    takeaways.push(Value::String(s.trim().to_string()));
                }
            }
        }
        if let Some(ch_arr) = part_val["chapters"].as_array() {
            for ch in ch_arr {
                let local_start = ch["start_time"]
                    .as_str()
                    .map(parse_timestamp_to_ms)
                    .unwrap_or(0);
                let local_end = ch["end_time"]
                    .as_str()
                    .map(parse_timestamp_to_ms)
                    .unwrap_or(clip_dur);
                let g_start = shift_clip_ts(local_start, s_ms, clip_dur, total_ms);
                let g_end = shift_clip_ts(local_end, s_ms, clip_dur, total_ms)
                    .max(g_start + 30_000)
                    .min(total_ms);

                let mut shifted_slides = Vec::new();
                if let Some(slides) = ch["slides"]
                    .as_array()
                    .or_else(|| ch["selected_slides"].as_array())
                {
                    for sl in slides {
                        if let Some(ts_str) =
                            sl["timestamp"].as_str().or_else(|| sl["time"].as_str())
                        {
                            let g_ts = shift_clip_ts(
                                parse_timestamp_to_ms(ts_str),
                                s_ms,
                                clip_dur,
                                total_ms,
                            );
                            let mut cloned = sl.clone();
                            cloned["timestamp"] = Value::String(format_timestamp_ms(g_ts));
                            shifted_slides.push(cloned);
                        }
                    }
                }
                let mut m_ch = ch.clone();
                m_ch["chapter_index"] = Value::from(merged_chapters.len() + 1);
                m_ch["start_ms"] = Value::from(g_start);
                m_ch["end_ms"] = Value::from(g_end);
                m_ch["slides"] = Value::Array(shifted_slides);
                merged_chapters.push(m_ch);
            }
        }
    }

    Ok(serde_json::json!({
        "article_title": first_title,
        "subtitle": first_sub,
        "executive_summary": summaries.join(" "),
        "key_takeaways": takeaways,
        "chapters": merged_chapters,
    }))
}

fn shift_clip_ts(ts_ms: u64, chunk_start_ms: u64, clip_dur_ms: u64, total_ms: u64) -> u64 {
    if chunk_start_ms > 0
        && ts_ms > clip_dur_ms + 15_000
        && ts_ms >= chunk_start_ms
        && ts_ms <= total_ms
    {
        ts_ms
    } else {
        (chunk_start_ms + ts_ms).min(total_ms)
    }
}

async fn materialize_gemini_plan(
    ffmpeg: &str,
    raw: &Value,
    probe: &VideoProbe,
    local_video: &Path,
    cap_dir: &Path,
) -> Result<(Stage1Plan, Vec<CandidateFrame>), VideoError> {
    let raw_chapters = raw["chapters"].as_array().cloned().unwrap_or_default();
    let mut requests: Vec<(usize, Value, u64)> = Vec::new();

    for (c_idx, ch) in raw_chapters.iter().enumerate() {
        if let Some(slides) = ch["slides"]
            .as_array()
            .or_else(|| ch["selected_slides"].as_array())
        {
            for sl in slides {
                if let Some(ts_str) = sl["timestamp"].as_str().or_else(|| sl["time"].as_str()) {
                    let mut ts_ms = parse_timestamp_to_ms(ts_str);
                    if probe.duration_ms > 1000 {
                        ts_ms = ts_ms.clamp(200, probe.duration_ms - 500);
                    }
                    requests.push((c_idx, sl.clone(), ts_ms));
                }
            }
        }
    }

    let all_ts: Vec<u64> = requests.iter().map(|x| x.2).collect();
    let captured =
        extract_frames_at_timestamps(ffmpeg, local_video, &all_ts, cap_dir, "hs").await?;

    let mut kept_frames: Vec<CandidateFrame> = Vec::new();
    let mut chapter_slides: HashMap<usize, Vec<Stage1SlideChoice>> = HashMap::new();

    for ((c_idx, sl_val, _), cand) in requests.into_iter().zip(captured) {
        let is_near_dup = kept_frames.iter().rev().take(3).any(|prev| {
            cand.timestamp_ms.abs_diff(prev.timestamp_ms) <= 45_000
                && hamming_distance(cand.dhash, prev.dhash) <= 5
        });
        if is_near_dup {
            let _ = std::fs::remove_file(&cand.image_path);
            continue;
        }
        chapter_slides
            .entry(c_idx)
            .or_default()
            .push(Stage1SlideChoice {
                frame_id: cand.frame_id.clone(),
                timestamp_ms: cand.timestamp_ms,
                role: sl_val["role"].as_str().unwrap_or("slide").to_string(),
                draft_caption: sl_val["draft_caption"]
                    .as_str()
                    .or_else(|| sl_val["caption"].as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        kept_frames.push(cand);
    }

    let mut chapters = Vec::new();
    for (c_idx, ch) in raw_chapters.iter().enumerate() {
        let start_ms = ch["start_ms"]
            .as_u64()
            .or_else(|| ch["start_time"].as_str().map(parse_timestamp_to_ms))
            .unwrap_or(0);
        let mut end_ms = ch["end_ms"]
            .as_u64()
            .or_else(|| ch["end_time"].as_str().map(parse_timestamp_to_ms))
            .unwrap_or(probe.duration_ms);
        if end_ms <= start_ms {
            end_ms = (start_ms + 30_000).max(probe.duration_ms);
        }
        let key_terms = ch["key_terms"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        chapters.push(Stage1ChapterPlan {
            chapter_index: c_idx + 1,
            title: ch["title"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("Chapter {}", c_idx + 1)),
            start_ms,
            end_ms,
            summary_goal: ch["summary_goal"].as_str().unwrap_or("").to_string(),
            key_terms,
            selected_slides: chapter_slides.remove(&c_idx).unwrap_or_default(),
        });
    }

    if chapters.is_empty() {
        chapters.push(Stage1ChapterPlan {
            chapter_index: 1,
            title: probe.title.clone(),
            start_ms: 0,
            end_ms: probe.duration_ms.max(60_000),
            summary_goal: "Complete presentation".into(),
            key_terms: Vec::new(),
            selected_slides: Vec::new(),
        });
    }

    let takeaways = raw["key_takeaways"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let plan = Stage1Plan {
        article_title: raw["article_title"]
            .as_str()
            .or_else(|| raw["title"].as_str())
            .unwrap_or(&probe.title)
            .to_string(),
        subtitle: raw["subtitle"].as_str().unwrap_or("").to_string(),
        executive_summary: raw["executive_summary"].as_str().unwrap_or("").to_string(),
        key_takeaways: takeaways,
        chapters,
    };

    Ok((plan, kept_frames))
}

#[allow(clippy::too_many_arguments)]
async fn run_local_frames_stage1<F>(
    ffmpeg: &str,
    probe: &VideoProbe,
    transcript: &TranscriptBundle,
    local_video: &Path,
    job_dir: &Path,
    stage1_ep: &StageEndpoint,
    target_language: &str,
    report: &F,
) -> Result<(Stage1Plan, Vec<CandidateFrame>), VideoError>
where
    F: Fn(PipelinePhase, u32, u32, &str, Option<String>),
{
    report(
        PipelinePhase::CapturingFrames,
        0,
        0,
        "Running local scene detection and slide deduplication…",
        Some(probe.title.clone()),
    );
    let candidates =
        build_local_candidate_frames(ffmpeg, local_video, probe, transcript, job_dir, 24).await?;
    let valid_map: HashMap<String, CandidateFrame> = candidates
        .iter()
        .map(|c| (c.frame_id.clone(), c.clone()))
        .collect();

    report(
        PipelinePhase::PlanningVisuals,
        0,
        0,
        &format!(
            "Selecting informative slides from {} candidates…",
            candidates.len()
        ),
        Some(probe.title.clone()),
    );

    let mut parts = Vec::new();
    let mut catalog = Vec::new();
    for c in &candidates {
        let img_p = c.thumb_path.as_deref().unwrap_or(&c.image_path);
        if let Ok(bytes) = std::fs::read(img_p) {
            parts.push(UserPart::Image {
                mime: "image/jpeg".into(),
                bytes,
            });
            catalog.push(format!(
                "- {}: timestamp={}, sharpness={:.1}",
                c.frame_id,
                format_timestamp_ms(c.timestamp_ms),
                c.sharpness
            ));
        }
    }

    let prompt = format!(
        "Target Output Language: {target_language}\nPresentation Title: {}\nDuration: {}\n\nCandidate Frame Catalog:\n{}\n\nTimestamped Transcript:\n{}",
        probe.title,
        format_timestamp_ms(probe.duration_ms),
        catalog.join("\n"),
        transcript.full_text()
    );
    parts.push(UserPart::Text(prompt));

    let http = build_streaming_http_client(240, 1200)?;
    let out = call_stage_model(
        &http,
        stage1_ep,
        LOCAL_FRAMES_STAGE1_SYSTEM,
        &parts,
        0.15,
        16384,
        true,
    )
    .await?;
    let raw = extract_json_object(&out.text)?;

    let mut chapters = Vec::new();
    if let Some(arr) = raw["chapters"].as_array() {
        for (idx, ch) in arr.iter().enumerate() {
            let start_ms = ch["start_time"]
                .as_str()
                .map(parse_timestamp_to_ms)
                .unwrap_or(0);
            let mut end_ms = ch["end_time"]
                .as_str()
                .map(parse_timestamp_to_ms)
                .unwrap_or(probe.duration_ms);
            if end_ms <= start_ms {
                end_ms = (start_ms + 30_000).max(probe.duration_ms);
            }
            let mut selected_slides = Vec::new();
            if let Some(sls) = ch["selected_slides"].as_array() {
                for sl in sls {
                    if let Some(fid) = sl["frame_id"].as_str()
                        && let Some(cand) = valid_map.get(fid)
                    {
                        selected_slides.push(Stage1SlideChoice {
                            frame_id: fid.to_string(),
                            timestamp_ms: cand.timestamp_ms,
                            role: sl["role"].as_str().unwrap_or("slide").to_string(),
                            draft_caption: sl["draft_caption"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
            }
            chapters.push(Stage1ChapterPlan {
                chapter_index: idx + 1,
                title: ch["title"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("Chapter {}", idx + 1)),
                start_ms,
                end_ms,
                summary_goal: ch["summary_goal"].as_str().unwrap_or("").to_string(),
                key_terms: ch["key_terms"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                selected_slides,
            });
        }
    }

    let plan = Stage1Plan {
        article_title: raw["article_title"]
            .as_str()
            .unwrap_or(&probe.title)
            .to_string(),
        subtitle: raw["subtitle"].as_str().unwrap_or("").to_string(),
        executive_summary: raw["executive_summary"].as_str().unwrap_or("").to_string(),
        key_takeaways: raw["key_takeaways"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        chapters,
    };

    Ok((plan, candidates))
}

#[allow(clippy::too_many_arguments)]
async fn write_single_chapter(
    http: &reqwest::Client,
    stage2_ep: &StageEndpoint,
    book_title: &str,
    total_chapters: usize,
    ch: &Stage1ChapterPlan,
    seg_text: &str,
    frame_lookup: &HashMap<String, CandidateFrame>,
    target_language: &str,
) -> Result<ArticleSection, VideoError> {
    let mut slide_lines = Vec::new();
    let mut slide_paths = Vec::new();
    for sl in &ch.selected_slides {
        if let Some(cand) = frame_lookup.get(&sl.frame_id) {
            slide_lines.push(format!(
                "- {} @ {} ({}): {}",
                sl.frame_id,
                format_timestamp_ms(cand.timestamp_ms),
                sl.role,
                sl.draft_caption
            ));
            let p = PathBuf::from(&cand.image_path);
            if p.is_file() {
                slide_paths.push(p);
            }
        }
    }

    // Cap attached vision images to 8 per chapter request to keep request payload bounded
    if slide_paths.len() > 8 {
        let step = slide_paths.len() as f64 / 8.0;
        slide_paths = (0..8)
            .map(|i| {
                let idx = ((i as f64 * step).round() as usize).min(slide_paths.len() - 1);
                slide_paths[idx].clone()
            })
            .collect();
    }

    let mut user_parts = Vec::new();
    for p in &slide_paths {
        if let Ok(bytes) = std::fs::read(p) {
            user_parts.push(UserPart::Image {
                mime: "image/jpeg".into(),
                bytes,
            });
        }
    }

    let terms = if ch.key_terms.is_empty() {
        "(auto)".to_string()
    } else {
        ch.key_terms.join(", ")
    };
    let slides_desc = if slide_lines.is_empty() {
        "(None assigned for this chapter)".to_string()
    } else {
        slide_lines.join("\n")
    };
    let user_prompt = format!(
        "Presentation: {book_title}\nChapter {} of {total_chapters}: {}\nTime Span: {} - {}\nEditorial Goal: {}\nKey Terms to Preserve: {terms}\n\nAssigned Slides for This Chapter ({}):\n{slides_desc}\n\nVerbatim Transcript Slice for This Chapter:\n{seg_text}",
        ch.chapter_index,
        ch.title,
        format_timestamp_ms(ch.start_ms),
        format_timestamp_ms(ch.end_ms),
        ch.summary_goal,
        ch.selected_slides.len()
    );
    user_parts.push(UserPart::Text(user_prompt));

    let sys = STAGE2_WRITER_SYSTEM.replace("{target_language}", target_language);
    let mut parsed_sec: Option<Value> = None;
    let mut last_err: Option<VideoError> = None;

    for attempt in 0..2 {
        let temp = if attempt == 0 { 0.25 } else { 0.1 };
        match call_stage_model(http, stage2_ep, &sys, &user_parts, temp, 12288, true).await {
            Ok(out) => match extract_json_object(&out.text) {
                Ok(val) => {
                    parsed_sec = Some(val);
                    break;
                }
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e),
        }
    }

    let parsed = match parsed_sec {
        Some(v) => v,
        None => {
            return Err(
                last_err.unwrap_or_else(|| VideoError::Format("Failed to write chapter.".into()))
            );
        }
    };

    let mut used_fids = HashSet::new();
    let mut blocks = parse_raw_blocks(&parsed["blocks"], frame_lookup, &mut used_fids);

    for sl in &ch.selected_slides {
        if !used_fids.contains(&sl.frame_id)
            && let Some(cand) = frame_lookup.get(&sl.frame_id)
        {
            blocks.push(ArticleBlock {
                kind: "figure".into(),
                frame_id: Some(sl.frame_id.clone()),
                image_path: Some(cand.image_path.clone()),
                timestamp_ms: Some(cand.timestamp_ms),
                caption: sl.draft_caption.clone(),
                ..Default::default()
            });
        }
    }

    Ok(ArticleSection {
        index: ch.chapter_index,
        title: parsed["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&ch.title)
            .to_string(),
        start_ms: ch.start_ms,
        end_ms: ch.end_ms,
        blocks,
    })
}

fn parse_raw_blocks(
    raw_blocks: &Value,
    frame_lookup: &HashMap<String, CandidateFrame>,
    used_fids: &mut HashSet<String>,
) -> Vec<ArticleBlock> {
    let Some(arr) = raw_blocks.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rb in arr {
        let btype = rb["type"]
            .as_str()
            .unwrap_or("paragraph")
            .trim()
            .to_ascii_lowercase();
        match btype.as_str() {
            "figure" => {
                let fid = rb["frame_id"].as_str().unwrap_or("").trim();
                if let Some(cand) = frame_lookup.get(fid) {
                    used_fids.insert(fid.to_string());
                    out.push(ArticleBlock {
                        kind: "figure".into(),
                        frame_id: Some(fid.to_string()),
                        image_path: Some(cand.image_path.clone()),
                        timestamp_ms: Some(cand.timestamp_ms),
                        caption: rb["caption"].as_str().unwrap_or("").trim().to_string(),
                        ..Default::default()
                    });
                }
            }
            "bullet_list" | "numbered_list" => {
                let items: Vec<String> = rb["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::trim))
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                if !items.is_empty() {
                    out.push(ArticleBlock {
                        kind: btype,
                        items,
                        ..Default::default()
                    });
                }
            }
            "code" => {
                let text = rb["text"]
                    .as_str()
                    .or_else(|| rb["code"].as_str())
                    .unwrap_or("")
                    .trim_matches('\n')
                    .to_string();
                if !text.is_empty() {
                    out.push(ArticleBlock {
                        kind: "code".into(),
                        language: rb["language"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                        text,
                        ..Default::default()
                    });
                }
            }
            "math" => {
                let tex = rb["tex"]
                    .as_str()
                    .or_else(|| rb["text"].as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !tex.is_empty() {
                    out.push(ArticleBlock {
                        kind: "math".into(),
                        tex: Some(tex.clone()),
                        text: tex,
                        ..Default::default()
                    });
                }
            }
            "table" => {
                let headers: Vec<String> = rb["headers"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let rows: Vec<Vec<String>> = rb["rows"]
                    .as_array()
                    .map(|rs| {
                        rs.iter()
                            .filter_map(|r| {
                                r.as_array().map(|cells| {
                                    cells
                                        .iter()
                                        .map(|c| c.as_str().unwrap_or("").to_string())
                                        .collect()
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !headers.is_empty() || !rows.is_empty() {
                    out.push(ArticleBlock {
                        kind: "table".into(),
                        headers,
                        rows,
                        ..Default::default()
                    });
                }
            }
            "subheading" | "quote" | "paragraph" => {
                let text = rb["text"].as_str().unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    out.push(ArticleBlock {
                        kind: btype,
                        text,
                        ..Default::default()
                    });
                }
            }
            _ => {
                let text = rb["text"].as_str().unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    out.push(ArticleBlock {
                        kind: "paragraph".into(),
                        text,
                        ..Default::default()
                    });
                }
            }
        }
    }
    out
}
