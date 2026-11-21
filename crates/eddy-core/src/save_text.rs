//! Text saved as it is: notes someone compiled, a Markdown file, an HTML
//! fragment. Unlike a saved page there is nothing to fetch or extract; what
//! is handed over is the document.

use eddy_doc::{ConvertOptions, html_to_document};
use eddy_model::{BlockKind, ContentOrigin, Document, Entity, fields, ids};
use eddy_store::ItemRow;

use crate::{Engine, EngineError, Result};

/// How the text given to [`Engine::save_text`] is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    Markdown,
    Html,
}

/// The longest text [`Engine::save_text`] takes.
pub const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

const TITLE_CHARS: usize = 80;

impl Engine {
    /// Saves `body` as a document of its own, in Saved.
    ///
    /// Without a `title` the first top-level heading names it, or failing
    /// that the opening words. `source_url` says where the text came from:
    /// relative links and images resolve against it. It does not become the
    /// item's address, so saving the page at that URL later still saves the
    /// page.
    ///
    /// The text is kept as HTML (Markdown is converted first), the form
    /// every device can turn into a document again.
    pub async fn save_text(
        &self,
        title: Option<&str>,
        body: &str,
        format: TextFormat,
        source_url: Option<&str>,
    ) -> Result<ItemRow> {
        if body.trim().is_empty() {
            return Err(EngineError::Invalid("没有可保存的文字。".into()));
        }
        if body.len() > MAX_TEXT_BYTES {
            return Err(EngineError::Invalid(format!(
                "文字太长了（上限 {} MB）。",
                MAX_TEXT_BYTES / (1024 * 1024)
            )));
        }
        let source_url = match source_url.map(str::trim).filter(|u| !u.is_empty()) {
            Some(raw) => match url::Url::parse(raw) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => Some(url.to_string()),
                _ => {
                    return Err(EngineError::Invalid(format!("不是有效的网址：{raw}")));
                }
            },
            None => None,
        };
        let html = match format {
            TextFormat::Markdown => eddy_extract::markdown_to_article_html(body),
            TextFormat::Html => body.to_string(),
        };
        // Converted once before anything is stored, to see that there is
        // something to keep and what to call it.
        let draft = html_to_document(
            &html,
            &ConvertOptions {
                base_url: source_url.as_deref(),
                title: None,
            },
        );
        if draft.blocks.is_empty() {
            return Err(EngineError::Invalid("没有可保存的文字。".into()));
        }
        let title = title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| title_of(&draft))
            .unwrap_or_else(|| "Untitled".to_string());

        let inner = &self.inner;
        let item_id = ids::new_local_id();
        let ingested = inner.ingest_html(
            &item_id,
            &html,
            ContentOrigin::Imported,
            source_url.as_deref(),
            Some(&title),
            true,
        )?;
        // The text exists nowhere else, so its content goes to other devices.
        inner.record(
            Entity::ContentVersion,
            &ingested.version.hash,
            fields::CREATE,
            serde_json::to_value(&ingested.version)?,
            false,
        )?;
        let row = inner.insert_saved(&item_id, None, title, &ingested)?;
        inner.auto_ai(vec![row.item.id.clone()]);
        Ok(row)
    }
}

/// The first top-level heading, or else how the text opens.
fn title_of(document: &Document) -> Option<String> {
    let heading = document.blocks.iter().find_map(|b| match &b.kind {
        BlockKind::Heading { level: 1, .. } => Some(b.plain_text()),
        _ => None,
    });
    let opening = || {
        document
            .blocks
            .iter()
            .filter(|b| !matches!(b.kind, BlockKind::Code { .. } | BlockKind::Math { .. }))
            .map(|b| b.plain_text())
            .find(|t| !t.trim().is_empty())
    };
    let text = heading.filter(|t| !t.trim().is_empty()).or_else(opening)?;
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut title: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        title.push('…');
    }
    Some(title)
}
