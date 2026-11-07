use std::collections::HashSet;

use eddy_model::{Block, BlockKind, Document, Locator, Position, TextQuote, utf16_len};

const QUOTE_CHARS: usize = 48;

fn weight(block: &Block) -> u32 {
    match block.kind {
        BlockKind::Image { .. }
        | BlockKind::Media { .. }
        | BlockKind::Math { .. }
        | BlockKind::Rule => 80,
        _ => utf16_len(&block.layout_text()).max(1),
    }
}

/// Fraction of the document before `offset` in block `block_id`.
pub fn progression_at(doc: &Document, block_id: &str, offset: u32) -> f64 {
    let total: u64 = doc.blocks.iter().map(|b| weight(b) as u64).sum();
    if total == 0 {
        return 0.0;
    }
    let mut before = 0u64;
    for block in &doc.blocks {
        let w = weight(block) as u64;
        if block.id == block_id {
            let within = if matches!(
                block.kind,
                BlockKind::Image { .. }
                    | BlockKind::Media { .. }
                    | BlockKind::Math { .. }
                    | BlockKind::Rule
            ) {
                0
            } else {
                (offset as u64).min(w)
            };
            return ((before + within) as f64 / total as f64).clamp(0.0, 1.0);
        }
        before += w;
    }
    0.0
}

/// Converts a UTF-16 offset into a char index within `text`.
fn char_index(text: &str, offset: u32) -> usize {
    let mut units = 0u32;
    for (i, c) in text.chars().enumerate() {
        if units >= offset {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    text.chars().count()
}

/// Builds a full locator for a position reported by a renderer.
pub fn locator_at(
    doc: &Document,
    content_hash: &str,
    block_id: &str,
    offset: u32,
) -> Option<Locator> {
    let idx = doc.blocks.iter().position(|b| b.id == block_id)?;
    let chars: Vec<char> = doc.blocks[idx].layout_text().chars().collect();
    let start = char_index(&chars.iter().collect::<String>(), offset).min(chars.len());
    let exact_end = (start + QUOTE_CHARS).min(chars.len());
    let exact: String = chars[start..exact_end].iter().collect();

    let mut prefix: String = chars[start.saturating_sub(QUOTE_CHARS)..start]
        .iter()
        .collect();
    if prefix.chars().count() < QUOTE_CHARS && idx > 0 {
        let prev: Vec<char> = doc.blocks[idx - 1].layout_text().chars().collect();
        let need = QUOTE_CHARS - prefix.chars().count();
        let tail: String = prev[prev.len().saturating_sub(need)..].iter().collect();
        prefix = format!("{tail}\n{prefix}");
    }
    let mut suffix: String = chars[exact_end..(exact_end + QUOTE_CHARS).min(chars.len())]
        .iter()
        .collect();
    if suffix.chars().count() < QUOTE_CHARS && idx + 1 < doc.blocks.len() {
        let next: String = doc.blocks[idx + 1]
            .layout_text()
            .chars()
            .take(QUOTE_CHARS - suffix.chars().count())
            .collect();
        suffix = format!("{suffix}\n{next}");
    }
    let clamped = utf16_len(&chars[..start].iter().collect::<String>());
    Some(Locator {
        content_hash: content_hash.to_string(),
        position: Position::Block {
            block_id: block_id.to_string(),
            offset: clamped,
        },
        quote: TextQuote {
            exact,
            prefix,
            suffix,
        },
        progression: progression_at(doc, block_id, clamped),
    })
}

/// Re-anchors a locator taken in another content version of the same item:
/// by block ID first, then by the quoted text, then by fuzzy similarity, and
/// finally by overall progression.
pub fn remap_locator(doc: &Document, content_hash: &str, loc: &Locator) -> Locator {
    if loc.content_hash == content_hash {
        return loc.clone();
    }
    if let Position::Block { block_id, offset } = &loc.position
        && let Some(block) = doc.block(block_id)
    {
        let offset = (*offset).min(utf16_len(&block.layout_text()));
        if let Some(l) = locator_at(doc, content_hash, block_id, offset) {
            return l;
        }
    }
    if let Some((block_id, offset)) = find_quote(doc, &loc.quote)
        && let Some(l) = locator_at(doc, content_hash, &block_id, offset)
    {
        return l;
    }
    if let Some(block_id) = fuzzy_block(doc, &loc.quote)
        && let Some(l) = locator_at(doc, content_hash, &block_id, 0)
    {
        return l;
    }
    let block_id = block_at_progression(doc, loc.progression);
    match block_id.and_then(|id| locator_at(doc, content_hash, &id, 0)) {
        Some(l) => l,
        None => Locator {
            content_hash: content_hash.to_string(),
            ..loc.clone()
        },
    }
}

fn find_quote(doc: &Document, quote: &TextQuote) -> Option<(String, u32)> {
    let exact = quote.exact.trim();
    if exact.chars().count() < 6 {
        return None;
    }
    let prefix_tail: String = {
        let p = quote.prefix.trim_end();
        let n = p.chars().count();
        p.chars().skip(n.saturating_sub(12)).collect()
    };
    let mut best: Option<(i32, String, u32)> = None;
    for block in &doc.blocks {
        let text = block.layout_text();
        let mut from = 0;
        while let Some(pos) = text[from..].find(exact) {
            let at = from + pos;
            let mut score = 1;
            if !prefix_tail.is_empty() && text[..at].trim_end().ends_with(prefix_tail.as_str()) {
                score += 2;
            }
            let offset = utf16_len(&text[..at]);
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, block.id.clone(), offset));
            }
            from = at + exact.len();
        }
    }
    best.map(|(_, id, off)| (id, off))
}

fn trigrams(s: &str) -> HashSet<[char; 3]> {
    let chars: Vec<char> = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

fn fuzzy_block(doc: &Document, quote: &TextQuote) -> Option<String> {
    let target = trigrams(&format!("{}{}{}", quote.prefix, quote.exact, quote.suffix));
    if target.len() < 4 {
        return None;
    }
    let mut best: Option<(f64, &str)> = None;
    for block in &doc.blocks {
        let text: String = block.layout_text().chars().take(2000).collect();
        let grams = trigrams(&text);
        if grams.is_empty() {
            continue;
        }
        let shared = target.intersection(&grams).count() as f64;
        let score = shared / target.len() as f64;
        if best.is_none_or(|b| score > b.0) {
            best = Some((score, &block.id));
        }
    }
    best.filter(|b| b.0 >= 0.35).map(|b| b.1.to_string())
}

fn block_at_progression(doc: &Document, progression: f64) -> Option<String> {
    let total: u64 = doc.blocks.iter().map(|b| weight(b) as u64).sum();
    let target = (progression.clamp(0.0, 1.0) * total as f64) as u64;
    let mut acc = 0u64;
    for block in &doc.blocks {
        acc += weight(block) as u64;
        if acc > target {
            return Some(block.id.clone());
        }
    }
    doc.blocks.last().map(|b| b.id.clone())
}
