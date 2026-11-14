use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eddy_model::{Entity, fields, ids, now_ms};
use eddy_store::VideoJobRecord;
use eddy_sync::{CreateVideoJobRequest, RemoteVideoJob};
use eddy_video::{
    PipelinePhase, PipelineProgress, ProtocolKind, RuntimeVideoConfig, StageEndpoint,
    ToolchainStatus, TranscriptPolicy, VideoError, VideoPipelineMode, VideoProbe, VideoSettings,
    VideoStageRole, detect_toolchain, probe_video, redact_text, run_video_pipeline,
    test_stage_endpoint,
};

use crate::engine::{Engine, Inner};
use crate::{EngineError, LogLevel, Result};

const VIDEO_SETTINGS_KEY: &str = "video";

pub type VideoJobCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoJobTarget {
    Local,
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoJobInfo {
    pub id: String,
    pub target: VideoJobTarget,
    pub source: String,
    pub title: String,
    pub pipeline: VideoPipelineMode,
    pub transcript_policy: TranscriptPolicy,
    pub target_language: String,
    pub phase: PipelinePhase,
    /// `queued`, `running`, `succeeded`, `failed`, `canceled`
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

#[derive(Clone, Debug)]
pub struct StartVideoJobOptions {
    pub source: String,
    pub target: VideoJobTarget,
    pub pipeline: Option<VideoPipelineMode>,
    pub transcript_policy: Option<TranscriptPolicy>,
    pub target_language: Option<String>,
    pub cookies_from_browser: Option<String>,
}

fn pipeline_to_str(p: VideoPipelineMode) -> &'static str {
    match p {
        VideoPipelineMode::Auto => "auto",
        VideoPipelineMode::GeminiTwoStage => "gemini_two_stage",
        VideoPipelineMode::LocalFramesTwoStage => "local_frames_two_stage",
    }
}

fn parse_pipeline_mode(s: &str) -> VideoPipelineMode {
    match s {
        "gemini_two_stage" => VideoPipelineMode::GeminiTwoStage,
        "local_frames_two_stage" => VideoPipelineMode::LocalFramesTwoStage,
        _ => VideoPipelineMode::Auto,
    }
}

fn policy_to_str(p: TranscriptPolicy) -> &'static str {
    match p {
        TranscriptPolicy::PreferManualSubtitles => "prefer_manual_subtitles",
        TranscriptPolicy::PreferAnySubtitles => "prefer_any_subtitles",
        TranscriptPolicy::AlwaysAsr => "always_asr",
    }
}

fn parse_transcript_policy(s: &str) -> TranscriptPolicy {
    match s {
        "prefer_manual_subtitles" => TranscriptPolicy::PreferManualSubtitles,
        "always_asr" => TranscriptPolicy::AlwaysAsr,
        _ => TranscriptPolicy::PreferAnySubtitles,
    }
}

fn record_to_info(r: VideoJobRecord, target: VideoJobTarget) -> VideoJobInfo {
    VideoJobInfo {
        id: r.id,
        target,
        source: r.source,
        title: r.title,
        pipeline: parse_pipeline_mode(&r.pipeline),
        transcript_policy: parse_transcript_policy(&r.transcript_policy),
        target_language: r.target_language,
        phase: PipelinePhase::parse(&r.phase),
        status: r.status,
        completed_chapters: r.completed_chapters,
        total_chapters: r.total_chapters,
        detail: r.detail,
        error: r.error,
        result_series_id: r.result_series_id,
        result_item_id: r.result_item_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn remote_to_info(r: RemoteVideoJob) -> VideoJobInfo {
    VideoJobInfo {
        id: r.id,
        target: VideoJobTarget::Server,
        source: r.source,
        title: r.title,
        pipeline: parse_pipeline_mode(&r.pipeline),
        transcript_policy: parse_transcript_policy(&r.transcript_policy),
        target_language: r.target_language,
        phase: PipelinePhase::parse(&r.phase),
        status: r.status,
        completed_chapters: r.completed_chapters,
        total_chapters: r.total_chapters,
        detail: r.detail,
        error: r.error,
        result_series_id: r.result_series_id,
        result_item_id: r.result_item_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

pub fn record_to_remote(r: VideoJobRecord) -> RemoteVideoJob {
    RemoteVideoJob {
        id: r.id,
        source: r.source,
        title: r.title,
        pipeline: r.pipeline,
        transcript_policy: r.transcript_policy,
        target_language: r.target_language,
        phase: r.phase,
        status: r.status,
        completed_chapters: r.completed_chapters,
        total_chapters: r.total_chapters,
        detail: r.detail,
        error: r.error,
        result_series_id: r.result_series_id,
        result_item_id: r.result_item_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

impl From<VideoError> for EngineError {
    fn from(e: VideoError) -> Self {
        match e {
            VideoError::Network(m) => EngineError::Network(m),
            other => EngineError::Invalid(other.to_string()),
        }
    }
}

impl Inner {
    pub(crate) fn video_jobs_dir(&self) -> PathBuf {
        self.data_dir.join("video-jobs")
    }

    pub(crate) fn notify_video_listener(&self) {
        if let Some(cb) = self.video_listener.read().clone() {
            cb();
        }
    }

    pub(crate) fn recover_legacy_video_setting(&self) -> Result<Option<VideoSettings>> {
        let ops = self.store.ops_since(0, 10_000)?;
        for (stored, _) in ops.into_iter().rev() {
            if stored.op.entity == Entity::Setting
                && stored.op.entity_id == VIDEO_SETTINGS_KEY
                && let Ok(mut parsed) = serde_json::from_value::<VideoSettings>(stored.op.value)
                && (!parsed.asr.model.is_empty()
                    || !parsed.stage1.model.is_empty()
                    || !parsed.stage2.model.is_empty()
                    || !parsed.stage2.url.is_empty())
            {
                if parsed.asr.model.trim().ends_with("-filetrans")
                    && parsed.asr.url.contains("compatible-mode")
                {
                    parsed.asr.model = "qwen3-asr-flash".into();
                }
                return Ok(Some(parsed));
            }
        }
        Ok(None)
    }

    /// Resolves a complete `RuntimeVideoConfig` on the client (combining `VideoSettings` with
    /// fallback `AiSettings` and Keychain keys) or on `eddy-server` (from `EDDY_VIDEO_*` env vars).
    pub(crate) fn resolve_runtime_video_config(
        &self,
        pipeline_override: Option<VideoPipelineMode>,
        policy_override: Option<TranscriptPolicy>,
        lang_override: Option<&str>,
        cookies_from_browser: Option<String>,
    ) -> Result<RuntimeVideoConfig> {
        if self.options.server_role {
            let mut cfg = RuntimeVideoConfig::from_env().ok_or_else(|| {
                EngineError::Invalid(
                    "Server video pipeline is not configured (set EDDY_VIDEO_STAGE2_* in deploy/.env)."
                        .into(),
                )
            })?;
            if let Some(p) = pipeline_override {
                cfg.pipeline = p;
            }
            if let Some(pol) = policy_override {
                cfg.transcript_policy = pol;
            }
            if let Some(lang) = lang_override.map(str::trim).filter(|s| !s.is_empty()) {
                cfg.target_language = lang.to_string();
            }
            return Ok(cfg);
        }

        let vs: VideoSettings = self
            .store
            .setting(VIDEO_SETTINGS_KEY)?
            .map(serde_json::from_value)
            .transpose()?
            .or_else(|| self.recover_legacy_video_setting().ok().flatten())
            .unwrap_or_default();
        let ai: eddy_ai::AiSettings = self
            .store
            .setting("ai")?
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();

        let fallback_url = ai
            .provider
            .as_ref()
            .map(|p| p.base_url.clone())
            .unwrap_or_default();
        let fallback_model = ai
            .provider
            .as_ref()
            .map(|p| p.model.clone())
            .unwrap_or_default();
        let fallback_key = self
            .platform
            .secret_get("ai:api-key")
            .unwrap_or_default();

        let resolve_ep = |role: VideoStageRole,
                          ep_cfg: &eddy_video::StageEndpointSettings,
                          inherit_model: bool|
         -> StageEndpoint {
            let url = if ep_cfg.url.trim().is_empty() {
                fallback_url.clone()
            } else {
                ep_cfg.url.trim().to_string()
            };
            let model = if ep_cfg.model.trim().is_empty() {
                if inherit_model {
                    fallback_model.clone()
                } else {
                    String::new()
                }
            } else {
                ep_cfg.model.trim().to_string()
            };
            let api_key = self
                .platform
                .secret_get(role.secret_key())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| fallback_key.clone());
            StageEndpoint {
                protocol: ep_cfg.protocol,
                url,
                model,
                api_key,
            }
        };

        let stage2 = resolve_ep(VideoStageRole::Stage2, &vs.stage2, true);
        if !stage2.is_configured() {
            return Err(EngineError::Invalid(
                "Configure Stage 2 (Chapter Writer) or the main AI provider in Settings -> Video first."
                    .into(),
            ));
        }

        let mut stage1 = resolve_ep(VideoStageRole::Stage1, &vs.stage1, false);
        if stage1.api_key.is_empty() {
            stage1.api_key = stage2.api_key.clone();
        }

        let mut vlm = resolve_ep(VideoStageRole::Vlm, &vs.vlm, false);
        if vlm.url.trim().is_empty() {
            vlm.url = stage2.url.clone();
        }
        if vlm.api_key.trim().is_empty() {
            vlm.api_key = stage2.api_key.clone();
        }
        if !vlm.is_configured() {
            vlm = if stage2.is_configured() {
                stage2.clone()
            } else {
                stage1.clone()
            };
        }

        let effective_mode = pipeline_override.unwrap_or(vs.pipeline);
        if effective_mode == VideoPipelineMode::GeminiTwoStage && !stage1.is_configured() {
            return Err(EngineError::Invalid(
                "Configure Stage 1A (Gemini Video Planner) in Settings -> Video, or switch pipeline to Local Scene Detection + VLM."
                    .into(),
            ));
        }

        let asr_ep = resolve_ep(VideoStageRole::Asr, &vs.asr, false);
        let asr = asr_ep.is_configured().then_some(asr_ep);

        let target_language = lang_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                let s = vs.target_language.trim();
                (!s.is_empty()).then(|| s.to_string())
            })
            .unwrap_or(ai.target_language);

        Ok(RuntimeVideoConfig {
            pipeline: effective_mode,
            transcript_policy: policy_override.unwrap_or(vs.transcript_policy),
            target_language,
            chapter_concurrency: vs.concurrency(),
            ffmpeg_path: (!vs.ffmpeg_path.trim().is_empty()).then(|| vs.ffmpeg_path.clone()),
            yt_dlp_path: (!vs.yt_dlp_path.trim().is_empty()).then(|| vs.yt_dlp_path.clone()),
            cookies_from_browser,
            asr,
            stage1,
            vlm,
            stage2,
        })
    }

    pub(crate) fn spawn_local_video_job(
        engine: Engine,
        job_id: String,
        cfg: RuntimeVideoConfig,
        on_server_change: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        engine
            .inner
            .video_cancels
            .lock()
            .insert(job_id.clone(), cancel.clone());

        tokio::spawn(async move {
            let inner = &engine.inner;
            let job_dir = inner.video_jobs_dir().join(&job_id);
            let Some(mut rec) = inner.store.video_job(&job_id).ok().flatten() else {
                inner.video_cancels.lock().remove(&job_id);
                return;
            };

            rec.status = "running".into();
            rec.error = None;
            rec.updated_at = now_ms();
            let _ = inner.store.upsert_video_job(&rec);
            inner.notify_video_listener();
            if let Some(ref notify) = on_server_change {
                notify();
            }

            let inner_prog = engine.inner.clone();
            let jid_prog = job_id.clone();
            let server_prog = on_server_change.clone();
            let progress_cb: eddy_video::ProgressCallback = Arc::new(move |p: PipelineProgress| {
                if let Ok(Some(mut cur)) = inner_prog.store.video_job(&jid_prog) {
                    if let Some(t) = p.resolved_title.filter(|t| !t.trim().is_empty()) {
                        cur.title = t;
                    }
                    cur.phase = p.phase.as_str().into();
                    cur.completed_chapters = p.completed_chapters;
                    cur.total_chapters = p.total_chapters;
                    cur.detail = p.detail;
                    cur.updated_at = now_ms();
                    let _ = inner_prog.store.upsert_video_job(&cur);
                    inner_prog.notify_video_listener();
                    if let Some(ref notify) = server_prog {
                        notify();
                    }
                }
            });

            let source = rec.source.clone();
            let outcome = run_video_pipeline(&source, &job_dir, &cfg, cancel.clone(), Some(progress_cb)).await;

            inner.video_cancels.lock().remove(&job_id);

            let Some(mut final_rec) = inner.store.video_job(&job_id).ok().flatten() else {
                return;
            };

            match outcome {
                Ok(generated) => match inner.import_epub_bytes(&generated.epub_bytes, &generated.title) {
                    Ok(imported) => {
                        final_rec.title = imported.title;
                        final_rec.phase = PipelinePhase::Completed.as_str().into();
                        final_rec.status = "succeeded".into();
                        final_rec.detail = format!("Imported {} chapters", generated.chapters_count);
                        final_rec.error = None;
                        final_rec.result_series_id = imported.series_id;
                        final_rec.result_item_id = Some(imported.item_id);
                        final_rec.updated_at = now_ms();
                        let _ = inner.store.upsert_video_job(&final_rec);

                        let _ = std::fs::remove_dir_all(&job_dir);

                        // In client mode connected to eddy-server, push the imported book ops + blob immediately.
                        if !inner.options.server_role && inner.hub_client().is_some() {
                            let eng = engine.clone();
                            tokio::spawn(async move {
                                let _ = eng.refresh().await;
                            });
                        }
                    }
                    Err(e) => {
                        final_rec.status = "failed".into();
                        final_rec.error = Some(redact_text(&e.to_string(), &cfg.all_secrets()));
                        final_rec.updated_at = now_ms();
                        let _ = inner.store.upsert_video_job(&final_rec);
                    }
                },
                Err(VideoError::Canceled) => {
                    final_rec.status = "canceled".into();
                    final_rec.detail = "Canceled".into();
                    final_rec.updated_at = now_ms();
                    let _ = inner.store.upsert_video_job(&final_rec);
                }
                Err(err) => {
                    let safe = redact_text(&err.to_string(), &cfg.all_secrets());
                    inner
                        .platform
                        .log(LogLevel::Warn, &format!("video job {job_id} failed: {safe}"));
                    final_rec.status = "failed".into();
                    final_rec.error = Some(safe);
                    final_rec.updated_at = now_ms();
                    let _ = inner.store.upsert_video_job(&final_rec);
                }
            }

            inner.notify_video_listener();
            if let Some(ref notify) = on_server_change {
                notify();
            }
        });
    }
}

impl Engine {
    pub fn set_video_listener(&self, listener: Option<VideoJobCallback>) {
        *self.inner.video_listener.write() = listener;
    }

    pub fn video_settings(&self) -> Result<VideoSettings> {
        if let Some(val) = self.inner.store.setting(VIDEO_SETTINGS_KEY)? {
            return Ok(serde_json::from_value(val)?);
        }
        // Recover any VideoSettings op previously written under field `"video"` before the
        // `fields::VALUE` fix, and persist it into `settings`.
        if let Some(recovered) = self.inner.recover_legacy_video_setting()? {
            self.inner.record(
                Entity::Setting,
                VIDEO_SETTINGS_KEY,
                fields::VALUE,
                serde_json::to_value(&recovered)?,
                false,
            )?;
            return Ok(recovered);
        }
        Ok(VideoSettings::default())
    }

    pub fn set_video_settings(&self, mut settings: VideoSettings) -> Result<()> {
        settings.chapter_concurrency = settings.chapter_concurrency.clamp(1, 6);
        // Normalize common DashScope async-only ASR slug to OpenAI-compatible `qwen3-asr-flash`
        if settings.asr.model.trim().ends_with("-filetrans")
            && settings.asr.url.contains("compatible-mode")
        {
            settings.asr.model = "qwen3-asr-flash".into();
        }
        self.inner.record(
            Entity::Setting,
            VIDEO_SETTINGS_KEY,
            fields::VALUE,
            serde_json::to_value(&settings)?,
            false,
        )?;
        Ok(())
    }

    pub fn set_video_stage_key(&self, role: VideoStageRole, api_key: &str) -> Result<()> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            self.inner.platform.secret_delete(role.secret_key());
            return Ok(());
        }
        if !self.inner.platform.secret_set(role.secret_key(), trimmed) {
            return Err(EngineError::Invalid(
                "Could not save the API key in your keychain.".into(),
            ));
        }
        Ok(())
    }

    pub fn has_video_stage_key(&self, role: VideoStageRole) -> bool {
        self.inner
            .platform
            .secret_get(role.secret_key())
            .is_some_and(|k| !k.trim().is_empty())
    }

    pub async fn test_video_stage(
        &self,
        role: VideoStageRole,
        protocol: ProtocolKind,
        url: &str,
        model: &str,
        api_key_override: Option<&str>,
    ) -> Result<u64> {
        let key = api_key_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| self.inner.platform.secret_get(role.secret_key()))
            .or_else(|| {
                (role == VideoStageRole::Vlm)
                    .then(|| self.inner.platform.secret_get(VideoStageRole::Stage2.secret_key()))
                    .flatten()
            })
            .or_else(|| self.inner.platform.secret_get("ai:api-key"))
            .unwrap_or_default();
        let ep = StageEndpoint {
            protocol,
            url: url.trim().to_string(),
            model: model.trim().to_string(),
            api_key: key,
        };
        Ok(test_stage_endpoint(&ep, role == VideoStageRole::Asr).await?)
    }

    pub fn check_video_toolchain(&self) -> Result<ToolchainStatus> {
        if self.inner.options.server_role {
            let cfg = RuntimeVideoConfig::from_env();
            return Ok(detect_toolchain(
                cfg.as_ref().and_then(|c| c.ffmpeg_path.as_deref()),
                cfg.as_ref().and_then(|c| c.yt_dlp_path.as_deref()),
            ));
        }
        let vs = self.video_settings()?;
        Ok(detect_toolchain(
            (!vs.ffmpeg_path.trim().is_empty()).then_some(vs.ffmpeg_path.as_str()),
            (!vs.yt_dlp_path.trim().is_empty()).then_some(vs.yt_dlp_path.as_str()),
        ))
    }

    /// True when running inside `eddy-server` with valid `EDDY_VIDEO_*` credentials and `ffmpeg`/`yt-dlp`.
    pub fn server_video_capable(&self) -> bool {
        RuntimeVideoConfig::from_env().is_some()
            && self
                .check_video_toolchain()
                .is_ok_and(|t| t.can_process_remote_urls())
    }

    pub async fn probe_video(
        &self,
        source: &str,
        cookies_from_browser: Option<&str>,
    ) -> Result<VideoProbe> {
        let toolchain = self.check_video_toolchain()?;
        Ok(probe_video(source, &toolchain, cookies_from_browser).await?)
    }

    pub async fn start_video_job(&self, opts: StartVideoJobOptions) -> Result<VideoJobInfo> {
        let source = opts.source.trim().to_string();
        if source.is_empty() {
            return Err(EngineError::Invalid("Enter a video URL or select a local video file.".into()));
        }

        if opts.target == VideoJobTarget::Server {
            if eddy_video::is_video_file_path(&source) && std::path::Path::new(&source).is_file() {
                return Err(EngineError::Invalid(
                    "Local video files run on This Mac so 4K frames are extracted locally without uploading gigabytes."
                        .into(),
                ));
            }
            let client = self.inner.hub_client().ok_or(EngineError::NotConfigured)?;
            let remote = client
                .create_video_job(&CreateVideoJobRequest {
                    source,
                    pipeline: opts.pipeline.map(|p| pipeline_to_str(p).into()),
                    transcript_policy: opts.transcript_policy.map(|p| policy_to_str(p).into()),
                    target_language: opts.target_language,
                })
                .await?;
            self.inner.notify_video_listener();
            return Ok(remote_to_info(remote));
        }

        self.enqueue_local_video_job(opts, None)
    }

    pub fn enqueue_local_video_job(
        &self,
        opts: StartVideoJobOptions,
        on_server_change: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<VideoJobInfo> {
        let cfg = self.inner.resolve_runtime_video_config(
            opts.pipeline,
            opts.transcript_policy,
            opts.target_language.as_deref(),
            opts.cookies_from_browser,
        )?;
        let now = now_ms();
        let initial_title = std::path::Path::new(opts.source.trim())
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|_| std::path::Path::new(opts.source.trim()).is_file())
            .unwrap_or(opts.source.trim())
            .to_string();

        let rec = VideoJobRecord {
            id: ids::new_local_id(),
            source: opts.source.trim().to_string(),
            title: initial_title,
            pipeline: pipeline_to_str(cfg.pipeline).into(),
            transcript_policy: policy_to_str(cfg.transcript_policy).into(),
            target_language: cfg.target_language.clone(),
            phase: PipelinePhase::Queued.as_str().into(),
            status: "queued".into(),
            completed_chapters: 0,
            total_chapters: 0,
            detail: "Queued…".into(),
            error: None,
            result_series_id: None,
            result_item_id: None,
            created_at: now,
            updated_at: now,
        };
        self.inner.store.upsert_video_job(&rec)?;
        Inner::spawn_local_video_job(self.clone(), rec.id.clone(), cfg, on_server_change);
        Ok(record_to_info(rec, VideoJobTarget::Local))
    }

    /// Lists local jobs merged with remote `eddy-server` jobs (when connected).
    pub async fn video_jobs(&self) -> Result<Vec<VideoJobInfo>> {
        let mut list: Vec<VideoJobInfo> = self
            .inner
            .store
            .video_jobs(50)?
            .into_iter()
            .map(|r| record_to_info(r, VideoJobTarget::Local))
            .collect();

        if !self.inner.options.server_role
            && let Some(client) = self.inner.hub_client()
            && let Ok(remote_jobs) = client.video_jobs().await
        {
            list.extend(remote_jobs.into_iter().map(remote_to_info));
        }

        list.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        Ok(list)
    }

    pub fn local_video_job_records(&self, limit: u32) -> Result<Vec<VideoJobRecord>> {
        Ok(self.inner.store.video_jobs(limit)?)
    }

    pub fn local_video_job_record(&self, id: &str) -> Result<Option<VideoJobRecord>> {
        Ok(self.inner.store.video_job(id)?)
    }

    pub async fn retry_video_job(&self, id: &str, target: VideoJobTarget) -> Result<VideoJobInfo> {
        if target == VideoJobTarget::Server {
            let client = self.inner.hub_client().ok_or(EngineError::NotConfigured)?;
            let remote = client.retry_video_job(id).await?;
            self.inner.notify_video_listener();
            return Ok(remote_to_info(remote));
        }
        self.retry_local_video_job(id, None)
    }

    pub fn retry_local_video_job(
        &self,
        id: &str,
        on_server_change: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<VideoJobInfo> {
        let mut rec = self
            .inner
            .store
            .video_job(id)?
            .ok_or(EngineError::NotFound)?;
        if rec.status == "running" {
            return Ok(record_to_info(rec, VideoJobTarget::Local));
        }
        let cfg = self.inner.resolve_runtime_video_config(
            Some(parse_pipeline_mode(&rec.pipeline)),
            Some(parse_transcript_policy(&rec.transcript_policy)),
            Some(&rec.target_language),
            None,
        )?;
        rec.status = "queued".into();
        rec.error = None;
        rec.detail = "Resuming from saved checkpoints…".into();
        rec.updated_at = now_ms();
        self.inner.store.upsert_video_job(&rec)?;
        Inner::spawn_local_video_job(self.clone(), rec.id.clone(), cfg, on_server_change);
        Ok(record_to_info(rec, VideoJobTarget::Local))
    }

    pub async fn cancel_video_job(&self, id: &str, target: VideoJobTarget) -> Result<VideoJobInfo> {
        if target == VideoJobTarget::Server {
            let client = self.inner.hub_client().ok_or(EngineError::NotConfigured)?;
            let remote = client.cancel_video_job(id).await?;
            self.inner.notify_video_listener();
            return Ok(remote_to_info(remote));
        }
        self.cancel_local_video_job(id)
    }

    pub fn cancel_local_video_job(&self, id: &str) -> Result<VideoJobInfo> {
        if let Some(flag) = self.inner.video_cancels.lock().get(id) {
            flag.store(true, Ordering::Relaxed);
        }
        let mut rec = self
            .inner
            .store
            .video_job(id)?
            .ok_or(EngineError::NotFound)?;
        rec.status = "canceled".into();
        rec.detail = "Canceled".into();
        rec.updated_at = now_ms();
        self.inner.store.upsert_video_job(&rec)?;
        self.inner.notify_video_listener();
        Ok(record_to_info(rec, VideoJobTarget::Local))
    }

    pub async fn dismiss_video_job(&self, id: &str, target: VideoJobTarget) -> Result<()> {
        if target == VideoJobTarget::Server {
            let client = self.inner.hub_client().ok_or(EngineError::NotConfigured)?;
            client.dismiss_video_job(id).await?;
            self.inner.notify_video_listener();
            return Ok(());
        }
        self.dismiss_local_video_job(id)
    }

    pub fn dismiss_local_video_job(&self, id: &str) -> Result<()> {
        if let Some(flag) = self.inner.video_cancels.lock().remove(id) {
            flag.store(true, Ordering::Relaxed);
        }
        self.inner.store.delete_video_job(id)?;
        let _ = std::fs::remove_dir_all(self.inner.video_jobs_dir().join(id));
        self.inner.notify_video_listener();
        Ok(())
    }
}
