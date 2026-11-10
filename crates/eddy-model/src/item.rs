use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// Article delivered by a subscription.
    Feed,
    /// Web page or document (PDF) saved by the user.
    Saved,
    /// Chapter of a series (multi-page tutorial, book).
    Chapter,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::Feed => "feed",
            ItemKind::Saved => "saved",
            ItemKind::Chapter => "chapter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "feed" => Some(ItemKind::Feed),
            "saved" => Some(ItemKind::Saved),
            "chapter" => Some(ItemKind::Chapter),
            _ => None,
        }
    }
}

/// An upstream service account. Its secret lives in the platform keychain
/// (standalone) or the server's encrypted store, never here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: String,
    pub base_url: String,
    pub username: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Feed,
    Saved,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Feed => "feed",
            SourceKind::Saved => "saved",
        }
    }
}

/// Per-source full-text policy: `Auto` lets truncation detection decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextMode {
    Auto,
    Always,
    Never,
}

impl FullTextMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            FullTextMode::Auto => "auto",
            FullTextMode::Always => "always",
            FullTextMode::Never => "never",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "always" => FullTextMode::Always,
            "never" => FullTextMode::Never,
            _ => FullTextMode::Auto,
        }
    }
}

/// Sources a full-text rule applies to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FullTextScope {
    All,
    Category { name: String },
    Source { id: String },
}

/// Sets the full-text mode for the sources in `scope`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextRule {
    pub scope: FullTextScope,
    pub mode: FullTextMode,
}

impl FullTextRule {
    pub fn matches(&self, source: &Source) -> bool {
        match &self.scope {
            FullTextScope::All => true,
            FullTextScope::Category { name } => source.category.as_deref() == Some(name.as_str()),
            FullTextScope::Source { id } => source.id == *id,
        }
    }
}

/// The source's mode under an ordered rule list: rules apply from the
/// bottom up, so the top-most matching rule wins. With none matching it is
/// the source's own setting.
pub fn resolve_full_text(source: &Source, rules: &[FullTextRule]) -> FullTextMode {
    rules
        .iter()
        .find(|r| r.matches(source))
        .map_or(source.full_text, |r| r.mode)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub account_id: Option<String>,
    pub upstream_id: Option<String>,
    pub kind: SourceKind,
    pub title: String,
    pub category: Option<String>,
    pub site_url: Option<String>,
    pub full_text: FullTextMode,
    /// Truncation detection concluded the feed only carries excerpts.
    pub truncated: bool,
    /// Shown in the Gallery view (the source's folder can also be).
    #[serde(default)]
    pub gallery: bool,
}

/// Snapshot of an item's metadata. Mutable reading state lives in its own
/// fields and is changed through operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub source_id: Option<String>,
    pub account_id: Option<String>,
    pub upstream_id: Option<String>,
    pub kind: ItemKind,
    pub url: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub published_at: i64,
    pub added_at: i64,
    pub read: bool,
    pub starred: bool,
    pub series_id: Option<String>,
    pub chapter_index: Option<u32>,
    /// Depth of the chapter in its series' table of contents: 0 for a
    /// top-level chapter, 1 for a section of one, and so on.
    #[serde(default)]
    pub chapter_level: u32,
    pub current_version: Option<String>,
    pub excerpt: Option<String>,
    pub lang: Option<String>,
    pub text_len: u32,
    pub progress: f64,
    pub finished: bool,
    pub last_read_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentOrigin {
    /// HTML delivered by the subscription.
    Feed,
    /// Full text extracted from the fetched page.
    Extracted,
    /// Page captured from the user's browser.
    Captured,
    /// File imported by the user (EPUB, PDF).
    Imported,
}

impl ContentOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentOrigin::Feed => "feed",
            ContentOrigin::Extracted => "extracted",
            ContentOrigin::Captured => "captured",
            ContentOrigin::Imported => "imported",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "feed" => Some(ContentOrigin::Feed),
            "extracted" => Some(ContentOrigin::Extracted),
            "captured" => Some(ContentOrigin::Captured),
            "imported" => Some(ContentOrigin::Imported),
            _ => None,
        }
    }
}

pub const EPUB_MIME: &str = "application/epub+zip";
pub const PDF_MIME: &str = "application/pdf";
/// URL scheme of files inside an imported EPUB: `epub://<blob key>/<path>`.
pub const EPUB_SCHEME: &str = "epub";
/// URL scheme of images inside an imported PDF:
/// `pdf://<blob key>/<page index>/<object index>`.
pub const PDF_SCHEME: &str = "pdf";

/// How an item's content is presented, decided by its current version's raw
/// source. Everything except PDF is a reflowable document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentFormat {
    #[default]
    Article,
    Pdf,
}

impl ContentFormat {
    pub fn from_mime(raw_mime: Option<&str>) -> Self {
        match raw_mime {
            Some(PDF_MIME) => ContentFormat::Pdf,
            _ => ContentFormat::Article,
        }
    }
}

/// An immutable rendition of an item's content. The raw source is always kept
/// so the document can be regenerated when the schema or extraction improves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentVersion {
    pub hash: String,
    pub item_id: String,
    pub origin: ContentOrigin,
    pub source_url: Option<String>,
    pub raw_blob: String,
    pub raw_mime: String,
    pub doc_blob: Option<String>,
    pub doc_schema: Option<u32>,
    pub text_len: u32,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub id: String,
    pub title: String,
    pub toc_url: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub author: Option<String>,
    /// Cover image URL (imported books).
    #[serde(default)]
    pub cover: Option<String>,
}
