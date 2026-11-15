use std::sync::Arc;

use eddy_core as core;

use crate::{EddyEngine, EddyError};

type R<T> = Result<T, EddyError>;

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoProtocolView {
    Auto,
    OpenAi,
    Gemini,
}

impl From<core::ProtocolKind> for VideoProtocolView {
    fn from(p: core::ProtocolKind) -> Self {
        match p {
            core::ProtocolKind::Auto => VideoProtocolView::Auto,
            core::ProtocolKind::OpenAi => VideoProtocolView::OpenAi,
            core::ProtocolKind::Gemini => VideoProtocolView::Gemini,
        }
    }
}

impl From<VideoProtocolView> for core::ProtocolKind {
    fn from(p: VideoProtocolView) -> Self {
        match p {
            VideoProtocolView::Auto => core::ProtocolKind::Auto,
            VideoProtocolView::OpenAi => core::ProtocolKind::OpenAi,
            VideoProtocolView::Gemini => core::ProtocolKind::Gemini,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPipelineModeView {
    Auto,
    GeminiTwoStage,
    LocalFramesTwoStage,
}

impl From<core::VideoPipelineMode> for VideoPipelineModeView {
    fn from(m: core::VideoPipelineMode) -> Self {
        match m {
            core::VideoPipelineMode::Auto => VideoPipelineModeView::Auto,
            core::VideoPipelineMode::GeminiTwoStage => VideoPipelineModeView::GeminiTwoStage,
            core::VideoPipelineMode::LocalFramesTwoStage => {
                VideoPipelineModeView::LocalFramesTwoStage
            }
        }
    }
}

impl From<VideoPipelineModeView> for core::VideoPipelineMode {
    fn from(m: VideoPipelineModeView) -> Self {
        match m {
            VideoPipelineModeView::Auto => core::VideoPipelineMode::Auto,
            VideoPipelineModeView::GeminiTwoStage => core::VideoPipelineMode::GeminiTwoStage,
            VideoPipelineModeView::LocalFramesTwoStage => {
                core::VideoPipelineMode::LocalFramesTwoStage
            }
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptPolicyView {
    PreferManualSubtitles,
    PreferAnySubtitles,
    AlwaysAsr,
}

impl From<core::TranscriptPolicy> for TranscriptPolicyView {
    fn from(p: core::TranscriptPolicy) -> Self {
        match p {
            core::TranscriptPolicy::PreferManualSubtitles => {
                TranscriptPolicyView::PreferManualSubtitles
            }
            core::TranscriptPolicy::PreferAnySubtitles => TranscriptPolicyView::PreferAnySubtitles,
            core::TranscriptPolicy::AlwaysAsr => TranscriptPolicyView::AlwaysAsr,
        }
    }
}

impl From<TranscriptPolicyView> for core::TranscriptPolicy {
    fn from(p: TranscriptPolicyView) -> Self {
        match p {
            TranscriptPolicyView::PreferManualSubtitles => {
                core::TranscriptPolicy::PreferManualSubtitles
            }
            TranscriptPolicyView::PreferAnySubtitles => core::TranscriptPolicy::PreferAnySubtitles,
            TranscriptPolicyView::AlwaysAsr => core::TranscriptPolicy::AlwaysAsr,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoStageRoleView {
    Asr,
    Stage1,
    Vlm,
    Stage2,
}

impl From<VideoStageRoleView> for core::VideoStageRole {
    fn from(r: VideoStageRoleView) -> Self {
        match r {
            VideoStageRoleView::Asr => core::VideoStageRole::Asr,
            VideoStageRoleView::Stage1 => core::VideoStageRole::Stage1,
            VideoStageRoleView::Vlm => core::VideoStageRole::Vlm,
            VideoStageRoleView::Stage2 => core::VideoStageRole::Stage2,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoJobTargetView {
    Local,
    Server,
}

impl From<core::VideoJobTarget> for VideoJobTargetView {
    fn from(t: core::VideoJobTarget) -> Self {
        match t {
            core::VideoJobTarget::Local => VideoJobTargetView::Local,
            core::VideoJobTarget::Server => VideoJobTargetView::Server,
        }
    }
}

impl From<VideoJobTargetView> for core::VideoJobTarget {
    fn from(t: VideoJobTargetView) -> Self {
        match t {
            VideoJobTargetView::Local => core::VideoJobTarget::Local,
            VideoJobTargetView::Server => core::VideoJobTarget::Server,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPhaseView {
    Queued,
    Probing,
    Transcribing,
    PlanningVisuals,
    CapturingFrames,
    WritingChapters,
    Packaging,
    Completed,
}

impl From<core::PipelinePhase> for VideoPhaseView {
    fn from(p: core::PipelinePhase) -> Self {
        match p {
            core::PipelinePhase::Queued => VideoPhaseView::Queued,
            core::PipelinePhase::Probing => VideoPhaseView::Probing,
            core::PipelinePhase::Transcribing => VideoPhaseView::Transcribing,
            core::PipelinePhase::PlanningVisuals => VideoPhaseView::PlanningVisuals,
            core::PipelinePhase::CapturingFrames => VideoPhaseView::CapturingFrames,
            core::PipelinePhase::WritingChapters => VideoPhaseView::WritingChapters,
            core::PipelinePhase::Packaging => VideoPhaseView::Packaging,
            core::PipelinePhase::Completed => VideoPhaseView::Completed,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VideoEndpointView {
    pub protocol: VideoProtocolView,
    pub url: String,
    pub model: String,
}

impl From<core::StageEndpointSettings> for VideoEndpointView {
    fn from(s: core::StageEndpointSettings) -> Self {
        VideoEndpointView {
            protocol: s.protocol.into(),
            url: s.url,
            model: s.model,
        }
    }
}

impl From<VideoEndpointView> for core::StageEndpointSettings {
    fn from(s: VideoEndpointView) -> Self {
        core::StageEndpointSettings {
            protocol: s.protocol.into(),
            url: s.url,
            model: s.model,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VideoSettingsView {
    pub pipeline: VideoPipelineModeView,
    pub transcript_policy: TranscriptPolicyView,
    pub target_language: String,
    pub chapter_concurrency: u32,
    pub ffmpeg_path: String,
    pub yt_dlp_path: String,
    pub asr: VideoEndpointView,
    pub stage1: VideoEndpointView,
    pub vlm: VideoEndpointView,
    pub stage2: VideoEndpointView,
}

impl From<core::VideoSettings> for VideoSettingsView {
    fn from(s: core::VideoSettings) -> Self {
        VideoSettingsView {
            pipeline: s.pipeline.into(),
            transcript_policy: s.transcript_policy.into(),
            target_language: s.target_language,
            chapter_concurrency: s.chapter_concurrency,
            ffmpeg_path: s.ffmpeg_path,
            yt_dlp_path: s.yt_dlp_path,
            asr: s.asr.into(),
            stage1: s.stage1.into(),
            vlm: s.vlm.into(),
            stage2: s.stage2.into(),
        }
    }
}

impl From<VideoSettingsView> for core::VideoSettings {
    fn from(s: VideoSettingsView) -> Self {
        core::VideoSettings {
            pipeline: s.pipeline.into(),
            transcript_policy: s.transcript_policy.into(),
            target_language: s.target_language,
            chapter_concurrency: s.chapter_concurrency,
            ffmpeg_path: s.ffmpeg_path,
            yt_dlp_path: s.yt_dlp_path,
            asr: s.asr.into(),
            stage1: s.stage1.into(),
            vlm: s.vlm.into(),
            stage2: s.stage2.into(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VideoToolchainView {
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    pub yt_dlp_path: Option<String>,
    pub ffmpeg_version: Option<String>,
    pub yt_dlp_version: Option<String>,
}

impl From<core::ToolchainStatus> for VideoToolchainView {
    fn from(t: core::ToolchainStatus) -> Self {
        VideoToolchainView {
            ffmpeg_path: t.ffmpeg_path,
            ffprobe_path: t.ffprobe_path,
            yt_dlp_path: t.yt_dlp_path,
            ffmpeg_version: t.ffmpeg_version,
            yt_dlp_version: t.yt_dlp_version,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VideoProbeView {
    pub video_id: String,
    pub source_url: Option<String>,
    pub local_video_path: Option<String>,
    pub is_local_file: bool,
    pub title: String,
    pub author: Option<String>,
    pub duration_ms: u64,
    pub chapters_count: u32,
    pub has_manual_subs: bool,
    pub has_auto_subs: bool,
}

impl From<core::VideoProbe> for VideoProbeView {
    fn from(p: core::VideoProbe) -> Self {
        let has_manual = p.has_manual_subs();
        let has_auto = p.has_auto_subs();
        VideoProbeView {
            video_id: p.video_id,
            source_url: p.source_url,
            local_video_path: p.local_video_path,
            is_local_file: p.is_local_file,
            title: p.title,
            author: p.author,
            duration_ms: p.duration_ms,
            chapters_count: p.chapters.len() as u32,
            has_manual_subs: has_manual,
            has_auto_subs: has_auto,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct StartVideoJobRequestView {
    pub source: String,
    pub target: VideoJobTargetView,
    pub pipeline: Option<VideoPipelineModeView>,
    pub transcript_policy: Option<TranscriptPolicyView>,
    pub target_language: Option<String>,
    pub cookies_from_browser: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct VideoJobView {
    pub id: String,
    pub target: VideoJobTargetView,
    pub source: String,
    pub title: String,
    pub pipeline: VideoPipelineModeView,
    pub transcript_policy: TranscriptPolicyView,
    pub target_language: String,
    pub phase: VideoPhaseView,
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

impl From<core::VideoJobInfo> for VideoJobView {
    fn from(j: core::VideoJobInfo) -> Self {
        VideoJobView {
            id: j.id,
            target: j.target.into(),
            source: j.source,
            title: j.title,
            pipeline: j.pipeline.into(),
            transcript_policy: j.transcript_policy.into(),
            target_language: j.target_language,
            phase: j.phase.into(),
            status: j.status,
            completed_chapters: j.completed_chapters,
            total_chapters: j.total_chapters,
            detail: j.detail,
            error: j.error,
            result_series_id: j.result_series_id,
            result_item_id: j.result_item_id,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

#[uniffi::export(with_foreign)]
pub trait VideoJobListener: Send + Sync {
    fn on_change(&self);
}

#[uniffi::export]
pub fn is_video_url(url: String) -> bool {
    core::is_video_url(&url)
}

#[uniffi::export]
pub fn is_video_file(path: String) -> bool {
    core::is_video_file_path(&path)
}

#[uniffi::export]
impl EddyEngine {
    pub fn set_video_job_listener(&self, listener: Option<Arc<dyn VideoJobListener>>) {
        let cb = listener.map(|l| Arc::new(move || l.on_change()) as core::VideoJobCallback);
        self.engine.set_video_listener(cb);
    }

    pub fn video_settings(&self) -> R<VideoSettingsView> {
        Ok(self.engine.video_settings()?.into())
    }

    pub fn set_video_settings(&self, settings: VideoSettingsView) -> R<()> {
        Ok(self.engine.set_video_settings(settings.into())?)
    }

    pub fn set_video_stage_key(&self, role: VideoStageRoleView, key: Option<String>) -> R<()> {
        Ok(self
            .engine
            .set_video_stage_key(role.into(), key.as_deref().unwrap_or(""))?)
    }

    pub fn has_video_stage_key(&self, role: VideoStageRoleView) -> bool {
        self.engine.has_video_stage_key(role.into())
    }

    pub fn check_video_toolchain(&self) -> R<VideoToolchainView> {
        Ok(self.engine.check_video_toolchain()?.into())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    pub async fn test_video_stage(
        &self,
        role: VideoStageRoleView,
        protocol: VideoProtocolView,
        url: String,
        model: String,
        key: Option<String>,
    ) -> R<u64> {
        Ok(self
            .engine
            .test_video_stage(role.into(), protocol.into(), &url, &model, key.as_deref())
            .await?)
    }

    pub async fn probe_video(
        &self,
        source: String,
        cookies_from_browser: Option<String>,
    ) -> R<VideoProbeView> {
        Ok(self
            .engine
            .probe_video(&source, cookies_from_browser.as_deref())
            .await?
            .into())
    }

    pub async fn start_video_job(&self, request: StartVideoJobRequestView) -> R<VideoJobView> {
        Ok(self
            .engine
            .start_video_job(core::StartVideoJobOptions {
                source: request.source,
                target: request.target.into(),
                pipeline: request.pipeline.map(Into::into),
                transcript_policy: request.transcript_policy.map(Into::into),
                target_language: request.target_language,
                cookies_from_browser: request.cookies_from_browser,
            })
            .await?
            .into())
    }

    pub async fn video_jobs(&self) -> R<Vec<VideoJobView>> {
        Ok(self
            .engine
            .video_jobs()
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn retry_video_job(&self, id: String, target: VideoJobTargetView) -> R<VideoJobView> {
        Ok(self
            .engine
            .retry_video_job(&id, target.into())
            .await?
            .into())
    }

    pub async fn cancel_video_job(
        &self,
        id: String,
        target: VideoJobTargetView,
    ) -> R<VideoJobView> {
        Ok(self
            .engine
            .cancel_video_job(&id, target.into())
            .await?
            .into())
    }

    pub async fn dismiss_video_job(&self, id: String, target: VideoJobTargetView) -> R<()> {
        Ok(self.engine.dismiss_video_job(&id, target.into()).await?)
    }
}
