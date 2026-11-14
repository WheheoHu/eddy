//! Video presentation to multi-chapter reflowable book pipeline (`eddy-video`).
//! Shared by both client (`eddy-core`) and optional server (`eddy-server`).

mod asr;
mod client;
mod epub_builder;
mod media;
mod models;
mod pipeline;
mod redact;
mod settings;
mod toolchain;

pub use client::{extract_json_object, test_stage_endpoint};
pub use epub_builder::build_epub_bytes;
pub use media::{compute_video_id, parse_vtt_or_srt, parse_youtube_json3, probe_video};
pub use models::{
    ArticleBlock, ArticleDocument, ArticleSection, CandidateFrame, ChapterHint, GeneratedBook,
    PipelinePhase, PipelineProgress, Stage1ChapterPlan, Stage1Plan, Stage1SlideChoice,
    TranscriptBundle, TranscriptSegment, VideoProbe, format_timestamp_ms, parse_timestamp_to_ms,
};
pub use pipeline::{ProgressCallback, run_video_pipeline};
pub use redact::{REDACTED, redact_text};
pub use settings::{
    ProtocolKind, ResolvedProtocol, RuntimeVideoConfig, StageEndpoint, StageEndpointSettings,
    TranscriptPolicy, VideoPipelineMode, VideoSettings, VideoStageRole,
};
pub use toolchain::{ToolchainStatus, detect_toolchain, is_video_file_path, is_video_url};

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Toolchain(String),
    #[error("{0}")]
    Media(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Format(String),
    #[error("Video conversion was canceled.")]
    Canceled,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests;
