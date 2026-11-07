use eddy_model::{BlockKind, Document};

const MARKERS: &[&str] = &[
    "read more",
    "continue reading",
    "read the full",
    "keep reading",
    "full story",
    "阅读全文",
    "查看全文",
    "继续阅读",
    "阅读原文",
    "点击查看",
    "全文链接",
];

/// Quick check on feed-provided content: does it look like an excerpt?
/// Short bodies and trailing "read more" style markers suggest so.
pub fn truncation_hint(doc: &Document) -> bool {
    if doc.text_len() < 300 {
        return true;
    }
    let tail: Vec<String> = doc
        .blocks
        .iter()
        .rev()
        .filter(|b| matches!(b.kind, BlockKind::Paragraph { .. } | BlockKind::ListItem { .. }))
        .take(2)
        .map(|b| b.plain_text().trim().to_lowercase())
        .collect();
    tail.iter().any(|t| {
        t.ends_with('…') || t.ends_with("...") || t.ends_with("[…]") || t.ends_with("[...]") || MARKERS.iter().any(|m| t.contains(m))
    })
}

/// Compares feed content with the extracted full text: the feed is
/// truncated when the page carries substantially more text.
pub fn is_truncated(feed_len: usize, full_len: usize) -> bool {
    full_len as f64 > feed_len as f64 * 1.5 + 200.0
}
