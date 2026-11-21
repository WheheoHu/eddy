//! Domain types shared by every Eddy crate: identifiers, the document model,
//! locators, items and the operation log.

pub mod doc;
pub mod hlc;
pub mod ids;
pub mod item;
pub mod locator;
pub mod markdown;
pub mod op;

pub use doc::*;
pub use hlc::{Hlc, HlcClock};
pub use item::*;
pub use locator::*;
pub use markdown::MarkdownOptions;
pub use op::*;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Length of `s` in UTF-16 code units, the offset unit shared with Swift and Kotlin.
pub fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}
