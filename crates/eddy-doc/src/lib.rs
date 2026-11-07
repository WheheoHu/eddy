//! Converts HTML into Eddy's reflowable document model and provides the text
//! utilities built on it: translation segments, locators and re-anchoring.

mod convert;
mod highlight;
mod locate;
mod math;

pub use convert::{ConvertOptions, html_to_document};
pub use highlight::highlight;
pub use locate::{locator_at, progression_at, remap_locator};
pub use math::{normalize_tex, tex_from_data_mw};

use eddy_model::{BlockKind, Document, runs_plain_text};

/// Version of the document model produced by this crate. Bump it whenever the
/// conversion output changes so stored documents are regenerated from their
/// raw source.
pub const SCHEMA_VERSION: u32 = 1;

/// A unit of text sent for translation, keyed so results map back to blocks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub id: String,
    pub text: String,
}

/// Translatable text in reading order. IDs are block IDs, with `#caption` for
/// image captions and `#rXcY` for table cells.
pub fn translation_segments(doc: &Document) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut push = |id: String, text: String| {
        let text = text.trim().to_string();
        if text.chars().any(|c| c.is_alphabetic()) {
            out.push(Segment { id, text });
        }
    };
    for block in &doc.blocks {
        match &block.kind {
            BlockKind::Heading { runs, .. } | BlockKind::Paragraph { runs } | BlockKind::ListItem { runs, .. } => {
                push(block.id.clone(), runs_plain_text(runs));
            }
            BlockKind::Image { caption, .. } if !caption.is_empty() => {
                push(format!("{}#caption", block.id), runs_plain_text(caption));
            }
            BlockKind::Table { rows } => {
                for (r, row) in rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        push(format!("{}#r{r}c{c}", block.id), runs_plain_text(&cell.runs));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests;
