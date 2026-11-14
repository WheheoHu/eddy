use serde::{Deserialize, Serialize};

pub fn format_timestamp_ms(ms: u64) -> String {
    let total_sec = (ms + 500) / 1000;
    let hours = total_sec / 3600;
    let rem = total_sec % 3600;
    let minutes = rem / 60;
    let seconds = rem % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub fn parse_timestamp_to_ms(raw: &str) -> u64 {
    let trimmed = raw
        .trim()
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')'));
    if trimmed.is_empty() {
        return 0;
    }
    let normalized = trimmed.replace(',', ".");
    if let Ok(sec) = normalized.parse::<f64>()
        && sec.is_finite()
        && sec >= 0.0
    {
        return (sec * 1000.0).round() as u64;
    }
    let parts: Vec<&str> = normalized.split(':').collect();
    match parts.as_slice() {
        [m, s] => {
            let mins = m.trim().parse::<u64>().unwrap_or(0);
            let secs = s.trim().parse::<f64>().unwrap_or(0.0).max(0.0);
            mins * 60_000 + (secs * 1000.0).round() as u64
        }
        [h, m, s] => {
            let hours = h.trim().parse::<u64>().unwrap_or(0);
            let mins = m.trim().parse::<u64>().unwrap_or(0);
            let secs = s.trim().parse::<f64>().unwrap_or(0.0).max(0.0);
            hours * 3_600_000 + mins * 60_000 + (secs * 1000.0).round() as u64
        }
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterHint {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoProbe {
    pub video_id: String,
    pub source_url: Option<String>,
    pub local_video_path: Option<String>,
    pub is_local_file: bool,
    pub title: String,
    pub author: Option<String>,
    pub duration_ms: u64,
    pub chapters: Vec<ChapterHint>,
    pub manual_sub_langs: Vec<String>,
    pub auto_sub_langs: Vec<String>,
}

impl VideoProbe {
    pub fn has_manual_subs(&self) -> bool {
        !self.manual_sub_langs.is_empty()
    }

    pub fn has_auto_subs(&self) -> bool {
        !self.auto_sub_langs.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub index: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptBundle {
    pub video_id: String,
    pub source: String,
    pub language: String,
    pub segments: Vec<TranscriptSegment>,
}

impl TranscriptBundle {
    pub fn full_text(&self) -> String {
        self.segments
            .iter()
            .filter(|s| !s.text.trim().is_empty())
            .map(|s| format!("[{}] {}", format_timestamp_ms(s.start_ms), s.text.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn slice_ms(&self, start_ms: u64, end_ms: u64, pad_ms: u64) -> Vec<&TranscriptSegment> {
        let lo = start_ms.saturating_sub(pad_ms);
        let hi = end_ms.saturating_add(pad_ms);
        let sliced: Vec<&TranscriptSegment> = self
            .segments
            .iter()
            .filter(|s| s.end_ms >= lo && s.start_ms <= hi && !s.text.trim().is_empty())
            .collect();
        if sliced.is_empty() {
            self.segments.iter().collect()
        } else {
            sliced
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateFrame {
    pub frame_id: String,
    pub timestamp_ms: u64,
    pub image_path: String,
    #[serde(default)]
    pub thumb_path: Option<String>,
    pub dhash: u64,
    pub sharpness: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage1SlideChoice {
    pub frame_id: String,
    pub timestamp_ms: u64,
    pub role: String,
    pub draft_caption: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage1ChapterPlan {
    pub chapter_index: usize,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub summary_goal: String,
    pub key_terms: Vec<String>,
    pub selected_slides: Vec<Stage1SlideChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage1Plan {
    pub article_title: String,
    pub subtitle: String,
    pub executive_summary: String,
    pub key_takeaways: Vec<String>,
    pub chapters: Vec<Stage1ChapterPlan>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArticleBlock {
    /// `paragraph`, `subheading`, `bullet_list`, `numbered_list`, `quote`, `code`, `math`, `table`, `figure`
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub items: Vec<String>,
    pub language: Option<String>,
    pub tex: Option<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub frame_id: Option<String>,
    pub image_path: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub caption: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleSection {
    pub index: usize,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub blocks: Vec<ArticleBlock>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleDocument {
    pub video_id: String,
    pub title: String,
    pub subtitle: String,
    pub author: Option<String>,
    pub source_url: Option<String>,
    pub duration_ms: u64,
    pub executive_summary: String,
    pub key_takeaways: Vec<String>,
    pub sections: Vec<ArticleSection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    Queued,
    Probing,
    Transcribing,
    PlanningVisuals,
    CapturingFrames,
    WritingChapters,
    Packaging,
    Completed,
}

impl PipelinePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            PipelinePhase::Queued => "queued",
            PipelinePhase::Probing => "probing",
            PipelinePhase::Transcribing => "transcribing",
            PipelinePhase::PlanningVisuals => "planning_visuals",
            PipelinePhase::CapturingFrames => "capturing_frames",
            PipelinePhase::WritingChapters => "writing_chapters",
            PipelinePhase::Packaging => "packaging",
            PipelinePhase::Completed => "completed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "probing" => PipelinePhase::Probing,
            "transcribing" => PipelinePhase::Transcribing,
            "planning_visuals" => PipelinePhase::PlanningVisuals,
            "capturing_frames" => PipelinePhase::CapturingFrames,
            "writing_chapters" => PipelinePhase::WritingChapters,
            "packaging" => PipelinePhase::Packaging,
            "completed" => PipelinePhase::Completed,
            _ => PipelinePhase::Queued,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineProgress {
    pub phase: PipelinePhase,
    pub completed_chapters: u32,
    pub total_chapters: u32,
    pub detail: String,
    pub resolved_title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GeneratedBook {
    pub title: String,
    pub author: Option<String>,
    pub chapters_count: u32,
    pub epub_bytes: Vec<u8>,
}
