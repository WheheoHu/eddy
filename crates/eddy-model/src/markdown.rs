//! A document as Markdown: what the command line hands to people and agents.
//!
//! The output is CommonMark with GFM tables and strikethrough. Formulas keep
//! their TeX between `$` (inline) and `$$` (display), which is also what the
//! document converter reads back, so text written out here can be saved again.

use std::ops::{Range, RangeInclusive};

use crate::doc::{Block, BlockKind, Document, Run, TableRow, runs_plain_text, style};

#[derive(Clone, Debug, Default)]
pub struct MarkdownOptions<'a> {
    /// Written as a top-level heading above the text.
    pub title: Option<&'a str>,
    /// Only these blocks, by position; see [`Document::page_blocks`].
    pub blocks: Option<Range<usize>>,
}

impl Document {
    pub fn to_markdown(&self, options: &MarkdownOptions) -> String {
        let blocks = match &options.blocks {
            Some(range) => {
                let end = range.end.min(self.blocks.len());
                &self.blocks[range.start.min(end)..end]
            }
            None => &self.blocks[..],
        };
        let mut out = String::new();
        if let Some(title) = options.title.map(str::trim).filter(|t| !t.is_empty()) {
            out.push_str("# ");
            out.push_str(&escape_text(&title.replace('\n', " ")));
        }
        let mut previous: Option<&Block> = None;
        for block in blocks {
            let body = render_block(block);
            if body.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
                // A blank line between blocks, inside the quote both are in.
                if !continues_list(previous, block) {
                    let shared = previous.map_or(0, |p| p.quote_depth.min(block.quote_depth));
                    out.push_str(quote_prefix(shared).trim_end());
                    out.push('\n');
                }
            }
            let prefix = quote_prefix(block.quote_depth);
            for (i, line) in body.split('\n').enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                if line.is_empty() {
                    out.push_str(prefix.trim_end());
                } else {
                    out.push_str(&prefix);
                    out.push_str(line);
                }
            }
            previous = Some(block);
        }
        out
    }

    /// Where the blocks of a PDF's pages are, for [`MarkdownOptions::blocks`].
    /// Pages count from 0. `None` when the document has no pages or the range
    /// runs past them.
    pub fn page_blocks(&self, pages: RangeInclusive<usize>) -> Option<Range<usize>> {
        let (first, last) = (*pages.start(), *pages.end());
        if first > last || last >= self.pages.len() {
            return None;
        }
        // A page with nothing on it starts where the next one does; the
        // pages after the last block start at the end.
        let start_of = |page: usize| {
            self.pages
                .get(page)
                .and_then(|id| self.blocks.iter().position(|b| &b.id == id))
                .unwrap_or(self.blocks.len())
        };
        let start = start_of(first);
        Some(start..start_of(last + 1).max(start))
    }
}

fn quote_prefix(depth: u8) -> String {
    "> ".repeat(depth as usize)
}

/// Whether `block` is the next item of the list `previous` is in, which
/// follows it without a blank line.
fn continues_list(previous: Option<&Block>, block: &Block) -> bool {
    match (previous.map(|p| &p.kind), &block.kind) {
        (Some(BlockKind::ListItem { .. }), BlockKind::ListItem { marker: true, .. }) => {
            previous.is_some_and(|p| p.quote_depth == block.quote_depth)
        }
        _ => false,
    }
}

/// One block without its quote prefix; empty when there is nothing to show.
fn render_block(block: &Block) -> String {
    match &block.kind {
        BlockKind::Heading { level, runs } => {
            let text = inline(runs).replace('\n', " ");
            let text = text.trim();
            if text.is_empty() {
                return String::new();
            }
            format!("{} {text}", "#".repeat((*level).clamp(1, 6) as usize))
        }
        BlockKind::Paragraph { runs } => lines(&inline(runs)).join("\\\n"),
        BlockKind::ListItem {
            ordered,
            depth,
            index,
            marker,
            runs,
        } => {
            let lines = lines(&inline(runs));
            if lines.is_empty() {
                return String::new();
            }
            let indent = "    ".repeat(*depth as usize);
            // Deep enough to belong to the item under any marker.
            let hang = format!("{indent}    ");
            let first = match (marker, ordered) {
                (false, _) => hang.clone(),
                (true, true) => format!("{indent}{index}. "),
                (true, false) => format!("{indent}- "),
            };
            let mut out = String::new();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    out.push_str("\\\n");
                }
                out.push_str(if i == 0 { &first } else { &hang });
                out.push_str(line);
            }
            out
        }
        BlockKind::Code { language, text, .. } => {
            let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
            let language = language
                .as_deref()
                .and_then(|l| l.split_whitespace().next())
                .filter(|l| !l.contains('`'))
                .unwrap_or_default();
            format!(
                "{fence}{language}\n{}\n{fence}",
                text.trim_end_matches('\n')
            )
        }
        BlockKind::Math { tex } => format!("$$\n{}\n$$", tex.trim()),
        BlockKind::Image {
            src, alt, caption, ..
        } => {
            let mut out = format!("![{}]({})", label(alt), link_target(src));
            let caption = lines(&inline(caption));
            if !caption.is_empty() {
                out.push('\n');
                out.push_str(&caption.join("\\\n"));
            }
            out
        }
        BlockKind::Table { rows } => table(rows),
        BlockKind::Media {
            src, poster, title, ..
        } => {
            let title = title.as_deref().map(str::trim).filter(|t| !t.is_empty());
            let target = link_target(src);
            match poster.as_deref().filter(|p| !p.is_empty()) {
                Some(poster) => format!(
                    "[![{}]({})]({target})",
                    label(title.unwrap_or_default()),
                    link_target(poster)
                ),
                None => format!("[{}]({target})", label(title.unwrap_or(src))),
            }
        }
        BlockKind::Rule => "---".to_string(),
    }
}

/// The lines of a paragraph, each safe to start a line with. A line break
/// inside the text becomes a hard break where the lines are joined.
fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| l.trim_matches([' ', '\t']))
        .filter(|l| !l.is_empty())
        .map(guard_line)
        .collect()
}

/// Escapes what would make the start of a line a heading, quote, list item,
/// rule or underline rather than text.
fn guard_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let space_at = |i: usize| bytes.get(i).is_none_or(|b| *b == b' ');
    let only = |allowed: &[u8]| bytes.iter().all(|b| allowed.contains(b));
    let at = match bytes.first() {
        Some(b'>') => Some(0),
        Some(b'#') => {
            let hashes = bytes.iter().take_while(|b| **b == b'#').count();
            (hashes <= 6 && space_at(hashes)).then_some(0)
        }
        Some(b'-') if space_at(1) || only(b"- ") => Some(0),
        Some(b'+') if space_at(1) => Some(0),
        Some(b'=') if only(b"= ") => Some(0),
        Some(d) if d.is_ascii_digit() => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            (matches!(bytes.get(digits), Some(b'.' | b')')) && space_at(digits + 1))
                .then_some(digits)
        }
        _ => None,
    };
    match at {
        Some(at) => format!("{}\\{}", &line[..at], &line[at..]),
        None => line.to_string(),
    }
}

/// Runs as inline Markdown. Line breaks in the text are left as they are for
/// the block to decide on.
fn inline(runs: &[Run]) -> String {
    fn link_of(run: &Run) -> Option<&str> {
        run.link.as_deref().filter(|l| !l.is_empty())
    }
    let mut out = String::new();
    let mut i = 0;
    while i < runs.len() {
        let link = link_of(&runs[i]);
        let mut j = i + 1;
        while j < runs.len() && link_of(&runs[j]) == link {
            j += 1;
        }
        let text = styled(&runs[i..j]);
        let core = text.trim();
        match link {
            // Space around the link's text stays outside the brackets.
            Some(url) if !core.is_empty() => {
                let start = text.len() - text.trim_start().len();
                out.push_str(&text[..start]);
                out.push('[');
                out.push_str(core);
                out.push_str("](");
                out.push_str(&link_target(url));
                out.push(')');
                out.push_str(&text[start + core.len()..]);
            }
            _ => out.push_str(&text),
        }
        i = j;
    }
    out
}

enum Piece<'a> {
    Text(&'a str),
    Space(&'a str),
    Code(&'a str),
    Math(&'a str),
}

/// Styles written with delimiters around the text, outermost first when
/// several open at once and last equally long.
const DELIMITED: [(u16, &str, &str); 8] = [
    (style::BOLD, "**", "**"),
    (style::ITALIC, "*", "*"),
    (style::STRIKE, "~~", "~~"),
    (style::UNDERLINE, "<u>", "</u>"),
    (style::MARK, "<mark>", "</mark>"),
    (style::SMALL, "<small>", "</small>"),
    (style::SUPERSCRIPT, "<sup>", "</sup>"),
    (style::SUBSCRIPT, "<sub>", "</sub>"),
];

fn delimiters(bit: u16) -> (&'static str, &'static str) {
    DELIMITED
        .iter()
        .find(|d| d.0 == bit)
        .map_or(("", ""), |d| (d.1, d.2))
}

/// Runs that share a link (or have none), with their styles as nested
/// delimiters.
fn styled(runs: &[Run]) -> String {
    let delimited = DELIMITED.iter().fold(0, |all, d| all | d.0);
    let mut pieces: Vec<(Piece, u16)> = Vec::new();
    for run in runs.iter().filter(|r| !r.text.is_empty()) {
        let bits = run.style & delimited;
        if run.math {
            pieces.push((Piece::Math(&run.text), bits));
        } else if run.style & style::CODE != 0 {
            pieces.push((Piece::Code(&run.text), bits));
        } else {
            // A delimiter only opens before a word and closes after one, so
            // the space at either end of a run is a piece of its own.
            let core = run.text.trim();
            let start = run.text.len() - run.text.trim_start().len();
            let end = start + core.len();
            if start > 0 {
                pieces.push((Piece::Space(&run.text[..start]), bits));
            }
            if !core.is_empty() {
                pieces.push((Piece::Text(core), bits));
                if end < run.text.len() {
                    pieces.push((Piece::Space(&run.text[end..]), bits));
                }
            }
        }
    }
    // Space is styled only as far as the words on both sides of it are.
    for i in 0..pieces.len() {
        if !matches!(pieces[i].0, Piece::Space(_)) {
            continue;
        }
        let word = |p: &&(Piece, u16)| !matches!(p.0, Piece::Space(_));
        let before = pieces[..i].iter().rev().find(word).map_or(0, |p| p.1);
        let after = pieces[i + 1..].iter().find(word).map_or(0, |p| p.1);
        pieces[i].1 = before & after;
    }

    let mut out = String::new();
    let mut open: Vec<u16> = Vec::new();
    for (i, (piece, bits)) in pieces.iter().enumerate() {
        let keep = open
            .iter()
            .position(|bit| bits & bit == 0)
            .unwrap_or(open.len());
        for bit in open.drain(keep..).rev() {
            out.push_str(delimiters(bit).1);
        }
        // The style that lasts longest opens first, so it closes last.
        let mut fresh: Vec<u16> = DELIMITED
            .iter()
            .map(|d| d.0)
            .filter(|bit| bits & bit != 0 && !open.contains(bit))
            .collect();
        fresh.sort_by_key(|bit| {
            std::cmp::Reverse(pieces[i..].iter().take_while(|p| p.1 & bit != 0).count())
        });
        for bit in fresh {
            out.push_str(delimiters(bit).0);
            open.push(bit);
        }
        match piece {
            Piece::Text(text) => out.push_str(&escape_text(text)),
            Piece::Space(space) => out.push_str(space),
            Piece::Code(code) => out.push_str(&code_span(code)),
            Piece::Math(tex) => {
                out.push('$');
                out.push_str(tex);
                out.push('$');
            }
        }
    }
    for bit in open.into_iter().rev() {
        out.push_str(delimiters(bit).1);
    }
    out
}

/// Escapes the characters that would be read as Markdown, and no more than
/// that: an underscore inside a word and a lone bracket stay as they are.
fn escape_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let previous = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let escape = match c {
            '\\' | '`' | '*' | '$' => true,
            '_' => {
                !(previous.is_some_and(char::is_alphanumeric)
                    && next.is_some_and(char::is_alphanumeric))
            }
            '~' => previous == Some('~') || next == Some('~'),
            // Only where it would close a link or a link definition.
            ']' => matches!(next, Some('(' | ':')),
            '<' => next.is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?')),
            '&' => next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#'),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The text of an image or media link, where every bracket counts.
fn label(text: &str) -> String {
    escape_text(&text.replace('\n', " "))
        .replace('[', "\\[")
        .replace("\\]", "]")
        .replace(']', "\\]")
}

fn link_target(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.trim().chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '\n' | '\r' | '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn code_span(text: &str) -> String {
    let text = text.replace('\n', " ");
    let fence = "`".repeat(longest_run(&text, '`') + 1);
    // Padding the parser strips again: needed beside a backtick, and when
    // the text has a space at both ends that would otherwise be stripped.
    let spaced = text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty();
    let pad = if text.starts_with('`') || text.ends_with('`') || spaced {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn longest_run(text: &str, of: char) -> usize {
    let (mut longest, mut current) = (0, 0);
    for c in text.chars() {
        current = if c == of { current + 1 } else { 0 };
        longest = longest.max(current);
    }
    longest
}

/// A pipe table, or HTML when a cell spans rows or columns, which a pipe
/// table cannot say.
fn table(rows: &[TableRow]) -> String {
    let columns = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    if rows
        .iter()
        .flat_map(|r| &r.cells)
        .any(|c| c.colspan > 1 || c.rowspan > 1)
    {
        return html_table(rows);
    }
    let line = |row: Option<&TableRow>| {
        let mut line = String::from("|");
        for i in 0..columns {
            let cell = row
                .and_then(|r| r.cells.get(i))
                .map(|c| {
                    inline(&c.runs)
                        .trim()
                        .replace('|', "\\|")
                        .replace('\n', "<br>")
                })
                .unwrap_or_default();
            line.push(' ');
            line.push_str(&cell);
            line.push_str(" |");
        }
        line
    };
    // A pipe table always has a header row; one without headings gets an
    // empty one rather than its first row promoted.
    let headed = rows[0].cells.iter().any(|c| c.header);
    let mut out = vec![line(headed.then_some(&rows[0]))];
    out.push(format!("|{}", " --- |".repeat(columns)));
    out.extend(rows[usize::from(headed)..].iter().map(|r| line(Some(r))));
    out.join("\n")
}

fn html_table(rows: &[TableRow]) -> String {
    let escape = |text: String| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br>")
    };
    let mut out = String::from("<table>\n");
    for row in rows {
        out.push_str("<tr>");
        for cell in &row.cells {
            let tag = if cell.header { "th" } else { "td" };
            out.push('<');
            out.push_str(tag);
            if cell.colspan > 1 {
                out.push_str(&format!(" colspan=\"{}\"", cell.colspan));
            }
            if cell.rowspan > 1 {
                out.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
            }
            out.push('>');
            out.push_str(&escape(runs_plain_text(&cell.runs)));
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{MediaKind, TableCell};

    fn block(kind: BlockKind) -> Block {
        Block {
            id: String::new(),
            quote_depth: 0,
            kind,
        }
    }

    fn quoted(depth: u8, kind: BlockKind) -> Block {
        Block {
            id: String::new(),
            quote_depth: depth,
            kind,
        }
    }

    fn doc(blocks: Vec<Block>) -> Document {
        let blocks = blocks
            .into_iter()
            .enumerate()
            .map(|(i, b)| Block {
                id: format!("b{i}"),
                ..b
            })
            .collect();
        Document {
            schema_version: 1,
            blocks,
            pages: Vec::new(),
        }
    }

    fn styled_run(text: &str, style: u16) -> Run {
        Run {
            style,
            ..Run::text(text)
        }
    }

    fn linked(text: &str, link: &str) -> Run {
        Run {
            link: Some(link.into()),
            ..Run::text(text)
        }
    }

    fn paragraph(runs: Vec<Run>) -> Block {
        block(BlockKind::Paragraph { runs })
    }

    fn item(ordered: bool, depth: u8, index: u32, marker: bool, text: &str) -> Block {
        block(BlockKind::ListItem {
            ordered,
            depth,
            index,
            marker,
            runs: vec![Run::text(text)],
        })
    }

    fn markdown(blocks: Vec<Block>) -> String {
        doc(blocks).to_markdown(&MarkdownOptions::default())
    }

    #[test]
    fn headings_and_paragraphs_are_separated_by_a_blank_line() {
        let out = markdown(vec![
            block(BlockKind::Heading {
                level: 2,
                runs: vec![Run::text("Tides")],
            }),
            paragraph(vec![Run::text("The sea rises.")]),
            block(BlockKind::Rule),
            paragraph(vec![Run::text("And falls.")]),
        ]);
        assert_eq!(out, "## Tides\n\nThe sea rises.\n\n---\n\nAnd falls.");
    }

    #[test]
    fn the_title_goes_on_top() {
        let out = doc(vec![paragraph(vec![Run::text("Body")])]).to_markdown(&MarkdownOptions {
            title: Some("A *starred* title"),
            blocks: None,
        });
        assert_eq!(out, "# A \\*starred\\* title\n\nBody");
    }

    #[test]
    fn styles_keep_their_space_outside_the_delimiters() {
        let out = markdown(vec![paragraph(vec![
            Run::text("a "),
            styled_run("bold ", style::BOLD),
            Run::text("c"),
        ])]);
        assert_eq!(out, "a **bold** c");
    }

    #[test]
    fn a_style_inside_another_nests() {
        let out = markdown(vec![paragraph(vec![
            styled_run("one ", style::BOLD),
            styled_run("two", style::BOLD | style::ITALIC),
            styled_run(" three", style::BOLD),
        ])]);
        assert_eq!(out, "**one *two* three**");
    }

    #[test]
    fn the_longer_style_is_the_outer_one() {
        let out = markdown(vec![paragraph(vec![
            styled_run("both", style::BOLD | style::ITALIC),
            styled_run(" italic", style::ITALIC),
        ])]);
        assert_eq!(out, "***both** italic*");
    }

    #[test]
    fn styles_without_markdown_use_html() {
        let out = markdown(vec![paragraph(vec![
            Run::text("H"),
            styled_run("2", style::SUBSCRIPT),
            Run::text("O is "),
            styled_run("wet", style::UNDERLINE | style::STRIKE),
        ])]);
        assert_eq!(out, "H<sub>2</sub>O is ~~<u>wet</u>~~");
    }

    #[test]
    fn links_wrap_their_runs() {
        let out = markdown(vec![paragraph(vec![
            Run::text("See "),
            linked("the ", "https://example.com/a b(1)"),
            Run {
                style: style::ITALIC,
                ..linked("guide", "https://example.com/a b(1)")
            },
            Run::text("."),
        ])]);
        assert_eq!(out, "See [the *guide*](https://example.com/a%20b%281%29).");
    }

    #[test]
    fn code_spans_fence_their_backticks() {
        let out = markdown(vec![paragraph(vec![
            styled_run("plain", style::CODE),
            Run::text(" and "),
            styled_run("a `tick`", style::CODE),
        ])]);
        assert_eq!(out, "`plain` and `` a `tick` ``");
    }

    #[test]
    fn formulas_keep_their_tex() {
        let out = markdown(vec![
            paragraph(vec![
                Run::text("Energy is "),
                Run::math("E = mc^2"),
                Run::text(", about $5."),
            ]),
            block(BlockKind::Math {
                tex: "\\int_0^1 x\\,dx".into(),
            }),
        ]);
        assert_eq!(
            out,
            "Energy is $E = mc^2$, about \\$5.\n\n$$\n\\int_0^1 x\\,dx\n$$"
        );
    }

    #[test]
    fn text_is_escaped_only_where_it_would_be_markup() {
        let out = markdown(vec![paragraph(vec![Run::text(
            "snake_case, _edge_, 2 * 3, [1] and [a](b), <div> but 1 < 2, AT&T &amp;",
        )])]);
        assert_eq!(
            out,
            "snake_case, \\_edge\\_, 2 \\* 3, [1] and [a\\](b), \\<div> but 1 < 2, AT\\&T \\&amp;"
        );
    }

    #[test]
    fn line_starts_that_look_like_blocks_are_escaped() {
        for (text, expected) in [
            ("# not a heading", "\\# not a heading"),
            ("#hashtag", "#hashtag"),
            ("> not a quote", "\\> not a quote"),
            ("- not a list", "\\- not a list"),
            ("-5 degrees", "-5 degrees"),
            ("---", "\\---"),
            ("1. not a list", "1\\. not a list"),
            ("2024. A year", "2024\\. A year"),
            ("3.14 is pi", "3.14 is pi"),
            ("+ plus", "\\+ plus"),
        ] {
            assert_eq!(markdown(vec![paragraph(vec![Run::text(text)])]), expected);
        }
    }

    #[test]
    fn a_line_break_is_a_hard_break() {
        let out = markdown(vec![paragraph(vec![Run::text("Roses are red\n- violets")])]);
        assert_eq!(out, "Roses are red\\\n\\- violets");
    }

    #[test]
    fn lists_nest_by_depth_and_stay_tight() {
        let out = markdown(vec![
            paragraph(vec![Run::text("Steps:")]),
            item(true, 0, 1, true, "First"),
            item(false, 1, 0, true, "detail"),
            item(false, 1, 0, false, "more about the detail"),
            item(true, 0, 2, true, "Second"),
            paragraph(vec![Run::text("Done.")]),
        ]);
        assert_eq!(
            out,
            "Steps:\n\n1. First\n    - detail\n\n        more about the detail\n2. Second\n\nDone."
        );
    }

    #[test]
    fn quotes_prefix_every_line() {
        let out = markdown(vec![
            quoted(
                1,
                BlockKind::Paragraph {
                    runs: vec![Run::text("Outer")],
                },
            ),
            quoted(
                2,
                BlockKind::Paragraph {
                    runs: vec![Run::text("Inner")],
                },
            ),
            quoted(
                1,
                BlockKind::Code {
                    language: None,
                    text: "a\n\nb".into(),
                    highlights: Vec::new(),
                },
            ),
            paragraph(vec![Run::text("After")]),
        ]);
        assert_eq!(
            out,
            "> Outer\n>\n> > Inner\n>\n> ```\n> a\n>\n> b\n> ```\n\nAfter"
        );
    }

    #[test]
    fn code_blocks_carry_their_language_and_outgrow_inner_fences() {
        let out = markdown(vec![
            block(BlockKind::Code {
                language: Some("rust".into()),
                text: "fn main() {}\n".into(),
                highlights: Vec::new(),
            }),
            block(BlockKind::Code {
                language: None,
                text: "```\nnested\n```".into(),
                highlights: Vec::new(),
            }),
        ]);
        assert_eq!(
            out,
            "```rust\nfn main() {}\n```\n\n````\n```\nnested\n```\n````"
        );
    }

    fn cell(text: &str, header: bool, colspan: u16) -> TableCell {
        TableCell {
            header,
            colspan,
            rowspan: 1,
            runs: vec![Run::text(text)],
        }
    }

    #[test]
    fn tables_are_pipe_tables() {
        let out = markdown(vec![block(BlockKind::Table {
            rows: vec![
                TableRow {
                    cells: vec![cell("Name", true, 1), cell("Use", true, 1)],
                },
                TableRow {
                    cells: vec![cell("a|b", false, 1), cell("two\nlines", false, 1)],
                },
                TableRow {
                    cells: vec![cell("short", false, 1)],
                },
            ],
        })]);
        assert_eq!(
            out,
            "| Name | Use |\n| --- | --- |\n| a\\|b | two<br>lines |\n| short |  |"
        );
    }

    #[test]
    fn a_table_without_headings_gets_an_empty_header() {
        let out = markdown(vec![block(BlockKind::Table {
            rows: vec![TableRow {
                cells: vec![cell("1", false, 1), cell("2", false, 1)],
            }],
        })]);
        assert_eq!(out, "|  |  |\n| --- | --- |\n| 1 | 2 |");
    }

    #[test]
    fn a_table_with_spans_falls_back_to_html() {
        let out = markdown(vec![block(BlockKind::Table {
            rows: vec![
                TableRow {
                    cells: vec![cell("Wide <b>", true, 2)],
                },
                TableRow {
                    cells: vec![cell("1", false, 1), cell("2", false, 1)],
                },
            ],
        })]);
        assert_eq!(
            out,
            "<table>\n<tr><th colspan=\"2\">Wide &lt;b&gt;</th></tr>\n<tr><td>1</td><td>2</td></tr>\n</table>"
        );
    }

    #[test]
    fn images_and_media_are_links() {
        let out = markdown(vec![
            block(BlockKind::Image {
                src: "https://example.com/a.png".into(),
                alt: "A [chart]".into(),
                width: None,
                height: None,
                caption: vec![Run::text("Figure 1")],
            }),
            block(BlockKind::Media {
                media: MediaKind::Video,
                src: "https://example.com/v.mp4".into(),
                poster: Some("https://example.com/p.jpg".into()),
                title: Some("Talk".into()),
            }),
            block(BlockKind::Media {
                media: MediaKind::Audio,
                src: "https://example.com/a.mp3".into(),
                poster: None,
                title: None,
            }),
        ]);
        assert_eq!(
            out,
            "![A \\[chart\\]](https://example.com/a.png)\nFigure 1\n\n\
             [![Talk](https://example.com/p.jpg)](https://example.com/v.mp4)\n\n\
             [https://example.com/a.mp3](https://example.com/a.mp3)"
        );
    }

    #[test]
    fn pages_select_their_blocks() {
        let mut document = doc(vec![
            paragraph(vec![Run::text("page one")]),
            paragraph(vec![Run::text("still one")]),
            paragraph(vec![Run::text("page three")]),
        ]);
        // The second page has nothing on it, so it starts where the third does.
        document.pages = vec!["b0".into(), "b2".into(), "b2".into()];
        assert_eq!(document.page_blocks(0..=0), Some(0..2));
        assert_eq!(document.page_blocks(1..=1), Some(2..2));
        assert_eq!(document.page_blocks(1..=2), Some(2..3));
        assert_eq!(document.page_blocks(0..=3), None);
        let out = document.to_markdown(&MarkdownOptions {
            title: None,
            blocks: document.page_blocks(2..=2),
        });
        assert_eq!(out, "page three");
    }
}
