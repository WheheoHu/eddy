use serde::{Deserialize, Serialize};
use url::Url;

use crate::VideoError;
use crate::redact::REDACTED;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    #[default]
    Auto,
    OpenAi,
    Gemini,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedProtocol {
    OpenAi,
    Gemini,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPipelineMode {
    #[default]
    Auto,
    GeminiTwoStage,
    LocalFramesTwoStage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptPolicy {
    PreferManualSubtitles,
    #[default]
    PreferAnySubtitles,
    AlwaysAsr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoStageRole {
    Asr,
    Stage1,
    Vlm,
    Stage2,
}

impl VideoStageRole {
    pub fn secret_key(self) -> &'static str {
        match self {
            VideoStageRole::Asr => "video:asr-key",
            VideoStageRole::Stage1 => "video:stage1-key",
            VideoStageRole::Vlm => "video:vlm-key",
            VideoStageRole::Stage2 => "video:stage2-key",
        }
    }
}

/// Non-secret stage endpoint configuration persisted in settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StageEndpointSettings {
    pub protocol: ProtocolKind,
    pub url: String,
    pub model: String,
}

/// Non-secret Video-to-Book settings. API keys stay in the local keychain
/// (or server environment variables).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    pub pipeline: VideoPipelineMode,
    pub transcript_policy: TranscriptPolicy,
    /// Empty inherits `AiSettings::target_language`.
    pub target_language: String,
    pub chapter_concurrency: u32,
    /// Optional custom binary path overrides.
    pub ffmpeg_path: String,
    pub yt_dlp_path: String,
    pub asr: StageEndpointSettings,
    /// Stage 1A: Gemini Native Video Planner (`/v1beta` full-video stream localization).
    pub stage1: StageEndpointSettings,
    /// Stage 1B: Vision Language Model (`VLM`) for `LocalFramesTwoStage` candidate slide selection.
    pub vlm: StageEndpointSettings,
    /// Stage 2: Deep Chapter Writer.
    pub stage2: StageEndpointSettings,
}

impl Default for VideoSettings {
    fn default() -> Self {
        VideoSettings {
            pipeline: VideoPipelineMode::Auto,
            transcript_policy: TranscriptPolicy::PreferAnySubtitles,
            target_language: String::new(),
            chapter_concurrency: 2,
            ffmpeg_path: String::new(),
            yt_dlp_path: String::new(),
            asr: StageEndpointSettings::default(),
            stage1: StageEndpointSettings::default(),
            vlm: StageEndpointSettings::default(),
            stage2: StageEndpointSettings::default(),
        }
    }
}

impl VideoSettings {
    pub fn concurrency(&self) -> usize {
        self.chapter_concurrency.clamp(1, 6) as usize
    }
}

/// Resolved runtime endpoint including its secret key (redacted on Debug).
#[derive(Clone)]
pub struct StageEndpoint {
    pub protocol: ProtocolKind,
    pub url: String,
    pub model: String,
    pub api_key: String,
}

impl std::fmt::Debug for StageEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageEndpoint")
            .field("protocol", &self.protocol)
            .field("url", &self.url)
            .field("model", &self.model)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "(empty)"
                } else {
                    REDACTED
                },
            )
            .finish()
    }
}

impl StageEndpoint {
    pub fn is_configured(&self) -> bool {
        !self.url.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.api_key.trim().is_empty()
    }

    pub fn resolved_protocol(&self) -> ResolvedProtocol {
        match self.protocol {
            ProtocolKind::OpenAi => return ResolvedProtocol::OpenAi,
            ProtocolKind::Gemini => return ResolvedProtocol::Gemini,
            ProtocolKind::Auto => {}
        }
        let raw = self.url.trim();
        if let Ok(parsed) = Url::parse(raw) {
            let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
            let path = parsed.path().trim_end_matches('/').to_ascii_lowercase();
            if path.ends_with("/openai")
                || path.contains("/v1beta/openai")
                || path.ends_with("/v1")
                || path.contains("/compatible-mode")
                || path.ends_with("/chat/completions")
                || path.ends_with("/audio/transcriptions")
            {
                return ResolvedProtocol::OpenAi;
            }
            if host.ends_with("googleapis.com")
                || path.ends_with("/v1beta")
                || path.ends_with("/v1alpha")
                || path.contains("/v1beta/")
                || path.contains("/gemini")
                || path.contains(":generatecontent")
                || path.contains(":streamgeneratecontent")
            {
                return ResolvedProtocol::Gemini;
            }
        }
        ResolvedProtocol::OpenAi
    }

    pub fn normalized_openai_base_url(&self) -> Result<String, VideoError> {
        let mut parsed = Url::parse(self.url.trim()).map_err(|_| {
            VideoError::Config("Enter a complete API URL, e.g. https://api.example.com/v1.".into())
        })?;
        if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
            return Err(VideoError::Config(
                "API URL must start with https:// or http://.".into(),
            ));
        }
        parsed.set_query(None);
        parsed.set_fragment(None);
        let mut path = parsed.path().trim_end_matches('/').to_string();
        for suffix in ["/chat/completions", "/audio/transcriptions"] {
            if let Some(stripped) = path.strip_suffix(suffix) {
                path = stripped.trim_end_matches('/').to_string();
            }
        }
        if path.is_empty() {
            path = "/v1".into();
        }
        parsed.set_path(&path);
        Ok(parsed.to_string().trim_end_matches('/').to_string())
    }

    pub fn normalized_gemini_base_url(&self) -> Result<String, VideoError> {
        let mut parsed = Url::parse(self.url.trim()).map_err(|_| {
            VideoError::Config(
                "Enter a complete Gemini API URL, e.g. https://generativelanguage.googleapis.com/v1beta."
                    .into(),
            )
        })?;
        if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
            return Err(VideoError::Config(
                "API URL must start with https:// or http://.".into(),
            ));
        }
        parsed.set_query(None);
        parsed.set_fragment(None);
        let mut path = parsed.path().trim_end_matches('/').to_string();
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
        parsed.set_path(&path);
        Ok(parsed.to_string().trim_end_matches('/').to_string())
    }

    pub fn gemini_generate_content_url(&self) -> Result<String, VideoError> {
        let base = self.normalized_gemini_base_url()?;
        let clean_model = self.model.trim().trim_start_matches("models/");
        if clean_model.is_empty() {
            return Err(VideoError::Config("Model ID is empty.".into()));
        }
        Ok(format!("{base}/models/{clean_model}:generateContent"))
    }

    pub fn gemini_stream_generate_content_url(&self) -> Result<String, VideoError> {
        let base = self.normalized_gemini_base_url()?;
        let clean_model = self.model.trim().trim_start_matches("models/");
        if clean_model.is_empty() {
            return Err(VideoError::Config("Model ID is empty.".into()));
        }
        Ok(format!(
            "{base}/models/{clean_model}:streamGenerateContent?alt=sse"
        ))
    }

    pub fn is_official_google_host(&self) -> bool {
        Url::parse(self.url.trim())
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
            .is_some_and(|h| h.ends_with("googleapis.com"))
    }
}

/// Complete runtime configuration passed into `run_video_pipeline`.
#[derive(Clone, Debug)]
pub struct RuntimeVideoConfig {
    pub pipeline: VideoPipelineMode,
    pub transcript_policy: TranscriptPolicy,
    pub target_language: String,
    pub chapter_concurrency: usize,
    pub ffmpeg_path: Option<String>,
    pub yt_dlp_path: Option<String>,
    pub cookies_from_browser: Option<String>,
    pub asr: Option<StageEndpoint>,
    /// Stage 1A: Gemini Native Video Planner.
    pub stage1: StageEndpoint,
    /// Stage 1B: Vision Language Model (VLM) for `LocalFramesTwoStage`.
    pub vlm: StageEndpoint,
    /// Stage 2: Deep Chapter Writer.
    pub stage2: StageEndpoint,
}

impl RuntimeVideoConfig {
    pub fn all_secrets(&self) -> Vec<&str> {
        let mut out = Vec::new();
        if let Some(asr) = &self.asr
            && !asr.api_key.is_empty()
        {
            out.push(asr.api_key.as_str());
        }
        if !self.stage1.api_key.is_empty() {
            out.push(self.stage1.api_key.as_str());
        }
        if !self.vlm.api_key.is_empty() {
            out.push(self.vlm.api_key.as_str());
        }
        if !self.stage2.api_key.is_empty() {
            out.push(self.stage2.api_key.as_str());
        }
        out
    }

    pub fn effective_pipeline(&self) -> VideoPipelineMode {
        match self.pipeline {
            VideoPipelineMode::GeminiTwoStage => VideoPipelineMode::GeminiTwoStage,
            VideoPipelineMode::LocalFramesTwoStage => VideoPipelineMode::LocalFramesTwoStage,
            VideoPipelineMode::Auto => {
                if self.stage1.is_configured()
                    && self.stage1.resolved_protocol() == ResolvedProtocol::Gemini
                {
                    VideoPipelineMode::GeminiTwoStage
                } else {
                    VideoPipelineMode::LocalFramesTwoStage
                }
            }
        }
    }

    /// Reads server-side `EDDY_VIDEO_*` environment variables (`deploy/.env`).
    pub fn from_env() -> Option<Self> {
        let env_str = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let parse_proto =
            |val: Option<String>| match val.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("openai") => ProtocolKind::OpenAi,
                Some("gemini") => ProtocolKind::Gemini,
                _ => ProtocolKind::Auto,
            };

        let default_url = env_str("EDDY_VIDEO_DEFAULT_URL").unwrap_or_default();
        let default_key = env_str("EDDY_VIDEO_DEFAULT_KEY").unwrap_or_default();

        let build_ep =
            |prefix: &str, fallback_url: &str, fallback_key: &str| -> Option<StageEndpoint> {
                let url = env_str(&format!("EDDY_VIDEO_{prefix}_URL"))
                    .unwrap_or_else(|| fallback_url.to_string());
                let model = env_str(&format!("EDDY_VIDEO_{prefix}_MODEL"))?;
                let api_key = env_str(&format!("EDDY_VIDEO_{prefix}_KEY"))
                    .unwrap_or_else(|| fallback_key.to_string());
                let protocol = parse_proto(env_str(&format!("EDDY_VIDEO_{prefix}_PROTOCOL")));
                let ep = StageEndpoint {
                    protocol,
                    url,
                    model,
                    api_key,
                };
                ep.is_configured().then_some(ep)
            };

        let stage2 = build_ep("STAGE2", &default_url, &default_key)?;
        let stage1 =
            build_ep("STAGE1", &stage2.url, &stage2.api_key).unwrap_or_else(|| stage2.clone());
        let vlm = build_ep("VLM", &stage2.url, &stage2.api_key).unwrap_or_else(|| stage2.clone());
        let asr = build_ep("ASR", &default_url, &default_key);

        let pipeline = match env_str("EDDY_VIDEO_PIPELINE")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("gemini_two_stage" | "gemini-two-stage") => VideoPipelineMode::GeminiTwoStage,
            Some("local_frames_two_stage" | "local-frames-two-stage" | "local") => {
                VideoPipelineMode::LocalFramesTwoStage
            }
            _ => VideoPipelineMode::Auto,
        };

        let transcript_policy = match env_str("EDDY_VIDEO_TRANSCRIPT_POLICY")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("manual" | "prefer_manual_subtitles") => TranscriptPolicy::PreferManualSubtitles,
            Some("always_asr" | "asr") => TranscriptPolicy::AlwaysAsr,
            _ => TranscriptPolicy::PreferAnySubtitles,
        };

        let chapter_concurrency = env_str("EDDY_VIDEO_CHAPTER_CONCURRENCY")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(2)
            .clamp(1, 6) as usize;

        Some(RuntimeVideoConfig {
            pipeline,
            transcript_policy,
            target_language: env_str("EDDY_VIDEO_TARGET_LANGUAGE")
                .unwrap_or_else(|| "简体中文".into()),
            chapter_concurrency,
            ffmpeg_path: env_str("EDDY_VIDEO_FFMPEG_PATH"),
            yt_dlp_path: env_str("EDDY_VIDEO_YT_DLP_PATH"),
            cookies_from_browser: None,
            asr,
            stage1,
            vlm,
            stage2,
        })
    }
}
