use serde::{Deserialize, Serialize};

/// A reflowable article: a flat list of typed blocks. Nesting (quotes, lists)
/// is expressed with depth fields so every renderer can map blocks straight to
/// paragraphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: u32,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub quote_depth: u8,
    pub kind: BlockKind,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    Heading {
        level: u8,
        runs: Vec<Run>,
    },
    Paragraph {
        runs: Vec<Run>,
    },
    /// `marker == false` continues the previous item without a bullet.
    ListItem {
        ordered: bool,
        depth: u8,
        index: u32,
        marker: bool,
        runs: Vec<Run>,
    },
    Code {
        language: Option<String>,
        text: String,
        highlights: Vec<HighlightSpan>,
    },
    Math {
        tex: String,
    },
    Image {
        src: String,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
        caption: Vec<Run>,
    },
    Table {
        rows: Vec<TableRow>,
    },
    Media {
        media: MediaKind,
        src: String,
        poster: Option<String>,
        title: Option<String>,
    },
    Rule,
}

impl BlockKind {
    pub fn tag(&self) -> &'static str {
        match self {
            BlockKind::Heading { .. } => "heading",
            BlockKind::Paragraph { .. } => "paragraph",
            BlockKind::ListItem { .. } => "list_item",
            BlockKind::Code { .. } => "code",
            BlockKind::Math { .. } => "math",
            BlockKind::Image { .. } => "image",
            BlockKind::Table { .. } => "table",
            BlockKind::Media { .. } => "media",
            BlockKind::Rule => "rule",
        }
    }

    pub fn runs(&self) -> Option<&[Run]> {
        match self {
            BlockKind::Heading { runs, .. }
            | BlockKind::Paragraph { runs }
            | BlockKind::ListItem { runs, .. } => Some(runs),
            BlockKind::Image { caption, .. } => Some(caption),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Embed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    pub header: bool,
    pub colspan: u16,
    pub rowspan: u16,
    pub runs: Vec<Run>,
}

/// Inline text with uniform style. A run with `math == true` holds LaTeX in
/// `text` and renders as a single attachment character.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub style: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub math: bool,
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

pub mod style {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const CODE: u16 = 1 << 2;
    pub const STRIKE: u16 = 1 << 3;
    pub const UNDERLINE: u16 = 1 << 4;
    pub const SUPERSCRIPT: u16 = 1 << 5;
    pub const SUBSCRIPT: u16 = 1 << 6;
    pub const SMALL: u16 = 1 << 7;
    pub const MARK: u16 = 1 << 8;
}

impl Run {
    pub fn text(text: impl Into<String>) -> Self {
        Run {
            text: text.into(),
            style: 0,
            link: None,
            math: false,
        }
    }

    pub fn math(tex: impl Into<String>) -> Self {
        Run {
            text: tex.into(),
            style: 0,
            link: None,
            math: true,
        }
    }
}

/// Syntax highlighting span over a code block, offsets in UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HighlightSpan {
    pub start: u32,
    pub end: u32,
    pub class: TokenClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenClass {
    Keyword,
    String,
    Comment,
    Number,
    Constant,
    Function,
    Type,
    Variable,
    Operator,
    Punctuation,
    Tag,
    Attribute,
}

/// The attachment character that stands in for an inline formula. Locator
/// offsets count each math run as this single character.
pub const OBJECT_REPLACEMENT: char = '\u{FFFC}';

impl Block {
    /// Text whose UTF-16 offsets match what renderers lay out: inline math
    /// collapses to one attachment character.
    pub fn layout_text(&self) -> String {
        match &self.kind {
            BlockKind::Code { text, .. } => text.clone(),
            BlockKind::Math { .. } => OBJECT_REPLACEMENT.to_string(),
            BlockKind::Table { rows } => rows
                .iter()
                .map(|r| {
                    r.cells
                        .iter()
                        .map(|c| runs_layout_text(&c.runs))
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            kind => kind.runs().map(runs_layout_text).unwrap_or_default(),
        }
    }

    /// Readable text for search, quotes, AI input: inline math keeps its LaTeX.
    pub fn plain_text(&self) -> String {
        match &self.kind {
            BlockKind::Code { text, .. } => text.clone(),
            BlockKind::Math { tex } => tex.clone(),
            BlockKind::Table { rows } => rows
                .iter()
                .map(|r| {
                    r.cells
                        .iter()
                        .map(|c| runs_plain_text(&c.runs))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            BlockKind::Image { alt, caption, .. } => {
                let caption = runs_plain_text(caption);
                if caption.is_empty() {
                    alt.clone()
                } else {
                    caption
                }
            }
            kind => kind.runs().map(runs_plain_text).unwrap_or_default(),
        }
    }
}

pub fn runs_layout_text(runs: &[Run]) -> String {
    runs.iter()
        .map(|r| {
            if r.math {
                OBJECT_REPLACEMENT.to_string()
            } else {
                r.text.clone()
            }
        })
        .collect()
}

pub fn runs_plain_text(runs: &[Run]) -> String {
    runs.iter()
        .map(|r| {
            if r.math {
                format!("${}$", r.text)
            } else {
                r.text.clone()
            }
        })
        .collect()
}

impl Document {
    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::plain_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Number of readable characters, used for word counts and truncation checks.
    pub fn text_len(&self) -> usize {
        self.blocks
            .iter()
            .filter(|b| !matches!(b.kind, BlockKind::Code { .. }))
            .map(|b| {
                b.plain_text()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .count()
            })
            .sum()
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// Opening text for lists and cards. Short formulas keep their symbols,
    /// long ones become an ellipsis, so raw TeX never fills the preview.
    pub fn excerpt(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            let runs = match &block.kind {
                BlockKind::Paragraph { runs } | BlockKind::ListItem { runs, .. } => runs,
                _ => continue,
            };
            let text: String = runs
                .iter()
                .map(|r| {
                    if !r.math {
                        r.text.clone()
                    } else if r.text.chars().count() <= 12 {
                        r.text
                            .chars()
                            .filter(|c| !matches!(c, '\\' | '{' | '}'))
                            .collect()
                    } else {
                        "…".to_string()
                    }
                })
                .collect();
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(text);
            if out.chars().count() >= max_chars {
                break;
            }
        }
        if out.chars().count() > max_chars {
            out = out.chars().take(max_chars).collect::<String>() + "…";
        }
        out
    }
}
