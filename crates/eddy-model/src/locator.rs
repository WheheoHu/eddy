use serde::{Deserialize, Serialize};

/// A format-independent reading position shared by progress, annotations and
/// cross-device resume.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    /// Content version the position was taken in.
    pub content_hash: String,
    pub position: Position,
    /// Text around the position, used to re-anchor in another content version.
    pub quote: TextQuote,
    /// Overall progress through the item, 0.0 ..= 1.0.
    pub progression: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Position {
    /// Reflowed article: block ID plus offset in UTF-16 code units of the
    /// block's layout text.
    Block { block_id: String, offset: u32 },
    Epub { href: String, progression: f64, cfi: Option<String> },
    Pdf { page: u32, progression: f64 },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextQuote {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}
