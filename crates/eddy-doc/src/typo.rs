//! Typographic analysis of mixed Chinese and Latin text. It only reports
//! where a renderer should act; the text itself is left as it is.

/// What a renderer does with a span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypoKind {
    /// Set in the Chinese font.
    Cjk,
    /// A full-width mark whose blank half meets the next mark's: pull the
    /// next character in by half an em.
    Squeeze,
    /// The last character before a Chinese/Latin boundary with no space in
    /// between: push the next character out by a quarter em.
    Gap,
}

/// A range of text in UTF-16 code units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypoSpan {
    pub start: u32,
    pub end: u32,
    pub kind: TypoKind,
}

/// Han, kana, CJK punctuation and full-width forms. Hangul is left out:
/// Korean separates words with spaces.
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{2e80}'..='\u{2fdf}'
        | '\u{3000}'..='\u{30ff}'
        | '\u{31c0}'..='\u{31ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{fe30}'..='\u{fe4f}'
        | '\u{ff00}'..='\u{ffef}'
        | '\u{20000}'..='\u{323af}')
}

/// Marks both scripts use; they follow the text around them.
fn is_shared(c: char) -> bool {
    matches!(c, '“' | '”' | '‘' | '’' | '—' | '…' | '·')
}

/// Ideographs and kana: the characters that want air next to Latin text.
fn is_ideograph(c: char) -> bool {
    is_cjk(c)
        && !matches!(c, '\u{3000}'..='\u{303f}' | '\u{fe30}'..='\u{fe4f}' | '\u{ff00}'..='\u{ffef}')
}

fn is_latin_word(c: char) -> bool {
    !is_cjk(c) && !is_shared(c) && c.is_alphanumeric()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Blank on the left: opening brackets and quotes.
    Open,
    /// Blank on the right: closing brackets, quotes, commas and stops.
    Close,
}

fn side(c: char) -> Option<Side> {
    // Curly quotes are left out: Chinese fonts on the Mac draw them at their
    // own narrow width, with no blank half to give up.
    match c {
        '「' | '『' | '（' | '《' | '〈' | '【' | '〔' | '〖' | '［' | '｛' => {
            Some(Side::Open)
        }
        '」' | '』' | '）' | '》' | '〉' | '】' | '〕' | '〗' | '］' | '｝' | '，' | '。'
        | '、' | '；' | '：' => Some(Side::Close),
        _ => None,
    }
}

/// Spans in reading order by kind: all `Cjk`, then `Squeeze`, then `Gap`.
pub fn analyze(text: &str) -> Vec<TypoSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut offsets = Vec::with_capacity(chars.len() + 1);
    let mut at = 0u32;
    for c in &chars {
        offsets.push(at);
        at += c.len_utf16() as u32;
    }
    offsets.push(at);

    // A shared mark is Chinese when the nearest other character on either
    // side is.
    let mut cjk: Vec<bool> = chars.iter().map(|&c| is_cjk(c)).collect();
    let mut i = 0;
    while i < chars.len() {
        if !is_shared(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_shared(chars[i]) {
            i += 1;
        }
        let before = start > 0 && cjk[start - 1];
        let after = i < chars.len() && cjk[i];
        if before || after {
            cjk[start..i].fill(true);
        }
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !cjk[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && cjk[i] {
            i += 1;
        }
        out.push(TypoSpan {
            start: offsets[start],
            end: offsets[i],
            kind: TypoKind::Cjk,
        });
    }
    let single = |i: usize, kind| TypoSpan {
        start: offsets[i],
        end: offsets[i + 1],
        kind,
    };
    let pairs = || (0..chars.len().saturating_sub(1)).filter(|&i| cjk[i] && cjk[i + 1]);
    for i in pairs() {
        let squeeze = matches!(
            (side(chars[i]), side(chars[i + 1])),
            (Some(Side::Close), Some(_)) | (Some(Side::Open), Some(Side::Open))
        );
        if squeeze {
            out.push(single(i, TypoKind::Squeeze));
        }
    }
    for i in 0..chars.len().saturating_sub(1) {
        let (a, b) = (chars[i], chars[i + 1]);
        if (is_ideograph(a) && is_latin_word(b)) || (is_latin_word(a) && is_ideograph(b)) {
            out.push(single(i, TypoKind::Gap));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str, kind: TypoKind) -> Vec<(u32, u32)> {
        analyze(text)
            .into_iter()
            .filter(|s| s.kind == kind)
            .map(|s| (s.start, s.end))
            .collect()
    }

    #[test]
    fn chinese_runs_include_their_punctuation() {
        assert_eq!(spans("Hi 潮起，潮落 ok", TypoKind::Cjk), vec![(3, 8)]);
        assert_eq!(spans("plain text", TypoKind::Cjk), vec![]);
    }

    #[test]
    fn shared_marks_follow_their_neighbours() {
        // Quotes around Chinese are Chinese; an apostrophe in English is not.
        assert_eq!(spans("他说“好”", TypoKind::Cjk), vec![(0, 5)]);
        assert_eq!(spans("it’s “fine” — yes", TypoKind::Cjk), vec![]);
        assert_eq!(spans("——节选", TypoKind::Cjk), vec![(0, 4)]);
    }

    #[test]
    fn adjacent_marks_are_squeezed() {
        // 》（ and ），: the first of each pair gives up its blank half.
        assert_eq!(
            spans("《潮起》（节选），完。", TypoKind::Squeeze),
            vec![(3, 4), (7, 8)]
        );
        assert_eq!(spans("《「引」》", TypoKind::Squeeze), vec![(0, 1), (3, 4)]);
        // An opening mark right after an opening-then-closing pair is untouched.
        assert_eq!(spans("（）", TypoKind::Squeeze), vec![]);
        assert_eq!(spans("你好，世界。", TypoKind::Squeeze), vec![]);
        // Curly quotes have no blank half.
        assert_eq!(spans("他说：“好。”", TypoKind::Squeeze), vec![]);
    }

    #[test]
    fn chinese_latin_boundaries_get_a_gap() {
        assert_eq!(spans("我的2024年终", TypoKind::Gap), vec![(1, 2), (5, 6)]);
        // Already spaced, or next to punctuation: nothing to add.
        assert_eq!(spans("用 Rust 写，（Swift）", TypoKind::Gap), vec![]);
    }

    #[test]
    fn offsets_are_utf16() {
        // The emoji is two code units.
        assert_eq!(spans("😀中a", TypoKind::Cjk), vec![(2, 3)]);
        assert_eq!(spans("😀中a", TypoKind::Gap), vec![(2, 3)]);
    }
}
