//! Reflows the lines and images of a PDF into a document using their
//! geometry.
//!
//! Running headers and footers (the same text in the page margins on
//! several pages) and bare page numbers are dropped. Columns are found
//! from gutters no line crosses and read one after another; a line or image
//! that spans the gutter (a title, a wide figure) splits the page into
//! bands. Headings are lines set larger than the body text, or short bold
//! lines. Paragraphs break at vertical gaps, first-line indents and short
//! last lines; hyphenation at line ends is undone and CJK lines join without
//! spaces.
//!
//! Images become figures where they sit in the reading order, moved to the
//! end of a paragraph they interrupt, with a "Figure N" line below as their
//! caption. Covers, full-page backgrounds, watermarks, logos repeated on
//! many pages, and small decorations are left out.

use std::collections::{HashMap, HashSet};

use eddy_doc::{ConvertOptions, html_to_document};
use eddy_model::{Document, PDF_SCHEME};

use crate::import::{PdfImage, PdfLine, PdfPage};

/// Fraction of the page height at top and bottom that counts as margin.
const MARGIN: f32 = 0.1;
/// A heading is at least this much larger than the body text.
const HEADING_SCALE: f32 = 1.15;
/// A bold heading at body size has at most this many characters.
const BOLD_HEADING_CHARS: usize = 120;
/// A line this much narrower than its column ends a paragraph.
const SHORT_LINE: f32 = 0.75;

/// The document of a PDF stored as blob `key`; images address it as
/// `pdf://<key>/<page>/<object>`.
pub(crate) fn document(pages: &[PdfPage], key: &str) -> Document {
    let html = reflow_html(pages);
    let base = format!("{PDF_SCHEME}://{key}/");
    html_to_document(
        &html,
        &ConvertOptions {
            base_url: Some(&base),
            title: None,
        },
    )
}

fn reflow_html(pages: &[PdfPage]) -> String {
    let body = body_size(pages);
    let repeated = repeated_margin_text(pages);
    let repeated_images = repeated_images(pages);
    let mut flow = Vec::new();
    for (n, page) in pages.iter().enumerate() {
        let lines: Vec<&PdfLine> = page
            .lines
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .filter(|l| !is_margin_junk(l, page, &repeated))
            .collect();
        let images: Vec<&PdfImage> = page
            .images
            .iter()
            .filter(|i| keeps_image(i, n, page, &lines, body, &repeated_images))
            .collect();
        // Labels drawn as text over a kept figure belong to the figure.
        let lines: Vec<&PdfLine> = lines
            .into_iter()
            .filter(|l| !images.iter().any(|i| is_label_of(l, i)))
            .collect();
        flow.extend(order_page(&lines, &images, n, page, body));
    }
    let flow = keep_paragraphs_whole(flow, body);
    let sizes = heading_sizes(&flow, body);
    let blocks = attach_captions(assemble(&flow, body, &sizes));
    blocks.iter().map(Block::html).collect()
}

/// Font size most characters are set in, to half a point.
fn body_size(pages: &[PdfPage]) -> f32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for line in pages.iter().flat_map(|p| &p.lines) {
        let key = (line.font_size * 2.0).round() as u32;
        if key > 0 {
            *counts.entry(key).or_default() += line.text.chars().count();
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(size, n)| (n, size))
        .map(|(size, _)| size as f32 / 2.0)
        .unwrap_or(10.0)
}

fn in_margin(line: &PdfLine, page: &PdfPage) -> bool {
    let top = line.y + line.height;
    line.y > page.height * (1.0 - MARGIN) || top < page.height * MARGIN
}

/// Text with digit runs collapsed, so "Page 3" and "Page 12" match.
fn normalized(text: &str) -> String {
    let mut out = String::new();
    let mut in_digits = false;
    for c in text.trim().to_lowercase().chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            if !c.is_whitespace() {
                out.push(c);
            }
        }
    }
    out
}

/// Margin texts that appear on three or more pages, or on most of them.
fn repeated_margin_text(pages: &[PdfPage]) -> Vec<String> {
    if pages.len() < 2 {
        return Vec::new();
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for page in pages {
        let mut seen: Vec<String> = Vec::new();
        for line in page.lines.iter().filter(|l| in_margin(l, page)) {
            let key = normalized(&line.text);
            if !key.is_empty() && !seen.contains(&key) {
                seen.push(key);
            }
        }
        for key in seen {
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|&(_, n)| n >= 3 || n * 2 > pages.len())
        .map(|(key, _)| key)
        .collect()
}

fn is_margin_junk(line: &PdfLine, page: &PdfPage, repeated: &[String]) -> bool {
    if !in_margin(line, page) {
        return false;
    }
    is_page_number(&line.text) || repeated.contains(&normalized(&line.text))
}

/// "12", "- 12 -", "Page 12", "12 of 30", "第 12 页", "xiv".
fn is_page_number(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let mut rest = trimmed.as_str();
    for prefix in ["page", "p.", "pg.", "第"] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.trim_start();
            break;
        }
    }
    if let Some(r) = rest.strip_suffix('页') {
        rest = r.trim_end();
    }
    if rest.is_empty() {
        return false;
    }
    let mut parts = rest.split(|c: char| c == '/' || c.is_whitespace()).filter(|s| !s.is_empty());
    let first = parts.next().unwrap_or("");
    let tail: Vec<&str> = parts.collect();
    let tail_ok = match tail.as_slice() {
        [] => true,
        [n] => is_number(n),
        ["of", n] => is_number(n),
        _ => false,
    };
    is_number(first) && tail_ok
}

fn is_number(s: &str) -> bool {
    !s.is_empty()
        && (s.chars().all(|c| c.is_ascii_digit())
            || s.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')))
}

/// Images drawn on three or more pages, or on most of them: logos,
/// watermarks, page decorations.
fn repeated_images(pages: &[PdfPage]) -> HashSet<u64> {
    if pages.len() < 2 {
        return HashSet::new();
    }
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for page in pages {
        let seen: HashSet<u64> = page.images.iter().map(|i| i.fingerprint).collect();
        for f in seen {
            *counts.entry(f).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|&(_, n)| n >= 3 || n * 2 > pages.len())
        .map(|(f, _)| f)
        .collect()
}

/// Whether an image is content worth showing rather than a cover,
/// background, watermark or decoration.
fn keeps_image(
    image: &PdfImage,
    page_index: usize,
    page: &PdfPage,
    lines: &[&PdfLine],
    body: f32,
    repeated: &HashSet<u64>,
) -> bool {
    let page_area = (page.width * page.height).max(1.0);
    // Only the part on the page counts; bleeds hang off the edges.
    let left = image.x.max(0.0);
    let right = (image.x + image.width).min(page.width);
    let bottom = image.y.max(0.0);
    let top = (image.y + image.height).min(page.height);
    let (width, height) = (right - left, top - bottom);
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let area = width * height;
    // Icons, bullets, spacers: tiny on the page or in pixels.
    if area < 36.0 * 36.0 || image.pixel_width * image.pixel_height < 64 * 64 {
        return false;
    }
    // Rules and bars.
    if width.max(height) > width.min(height) * 15.0 {
        return false;
    }
    if area >= page_area * 0.85 || repeated.contains(&image.fingerprint) {
        return false;
    }
    if top < page.height * MARGIN || bottom > page.height * (1.0 - MARGIN) {
        return false;
    }
    // A cover or title page: pictures on a first page with little text.
    if page_index == 0 && lines.len() < 12 {
        return false;
    }
    // A background or watermark under running text.
    let text_over = lines
        .iter()
        .filter(|l| {
            center_inside(l, image)
                && l.width >= width * 0.5
                && (l.font_size - body).abs() <= body * 0.15
        })
        .count();
    text_over < 3
}

fn center_inside(line: &PdfLine, image: &PdfImage) -> bool {
    let cx = line.x + line.width / 2.0;
    let cy = line.y + line.height / 2.0;
    cx > image.x && cx < image.x + image.width && cy > image.y && cy < image.y + image.height
}

/// A line drawn wholly on a figure: an axis label, a callout. A caption
/// placed over the picture's edge is still the caption.
fn is_label_of(line: &PdfLine, image: &PdfImage) -> bool {
    line.x >= image.x
        && line.x + line.width <= image.x + image.width
        && line.y >= image.y
        && line.y + line.height <= image.y + image.height
        && !is_caption(&line.text)
}

/// "Figure 3", "Fig. 3", "[Figure 2.1.A]", "图 3": the caption of the
/// figure above.
fn is_caption(text: &str) -> bool {
    let text = text.trim_start().trim_start_matches(['[', '(']);
    ["figure", "fig.", "fig ", "plate", "chart", "图", "圖"]
        .iter()
        .any(|prefix| {
            let Some(head) = text.get(..prefix.len()) else {
                return false;
            };
            head.to_lowercase() == *prefix
                && text[prefix.len()..].trim_start().chars().next().is_some_and(|c| {
                    c.is_ascii_digit() || c.is_ascii_uppercase() || "一二三四五六七八九十".contains(c)
                })
        })
}

/// A line in reading order with the shape of the column it sits in.
struct Placed<'a> {
    line: &'a PdfLine,
    column_width: f32,
    /// Where most lines of the column start; a line to the right of it is
    /// indented, one to the left is outdented (a hanging indent).
    column_x: f32,
    /// The paragraph continues into this line across a figure, so the
    /// vertical gap before it is not a paragraph space.
    joined: bool,
}

/// What reads next: a line of text or a figure.
enum Flow<'a> {
    Text(Placed<'a>),
    Image { image: &'a PdfImage, page: usize },
}

impl<'a> Flow<'a> {
    fn text(&self) -> Option<&Placed<'a>> {
        match self {
            Flow::Text(p) => Some(p),
            Flow::Image { .. } => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Piece<'a> {
    Line(&'a PdfLine),
    Image(&'a PdfImage),
}

impl Piece<'_> {
    /// Left, bottom, width, height.
    fn rect(&self) -> (f32, f32, f32, f32) {
        match self {
            Piece::Line(l) => (l.x, l.y, l.width, l.height),
            Piece::Image(i) => (i.x, i.y, i.width, i.height),
        }
    }
}

/// The left edge most lines share, to the point.
fn dominant_x(lines: &[&PdfLine]) -> f32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for l in lines {
        *counts.entry(l.x.round() as i32).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(x, n)| (n, -x))
        .map(|(x, _)| x as f32)
        .unwrap_or(0.0)
}

/// Reading order for one page: columns between gutters, read one after
/// another within each band that spanning lines and images delimit.
fn order_page<'a>(
    lines: &[&'a PdfLine],
    images: &[&'a PdfImage],
    page_index: usize,
    page: &PdfPage,
    body: f32,
) -> Vec<Flow<'a>> {
    let gutters = find_gutters(lines, images, page, body);
    let column_of = |piece: &Piece| -> Option<usize> {
        let (x, _, width, _) = piece.rect();
        let right = x + width;
        // A piece spans when it reaches past a gutter on both sides.
        if gutters
            .iter()
            .any(|&(gl, gr)| x < gl - body && right > gr + body)
        {
            return None;
        }
        let center = x + width / 2.0;
        Some(gutters.iter().filter(|&&(_, gr)| center > gr).count())
    };

    let mut pieces: Vec<Piece<'a>> = lines
        .iter()
        .map(|l| Piece::Line(l))
        .chain(images.iter().map(|i| Piece::Image(i)))
        .collect();
    // Top-down by 2-point rows; within a row, a title set flush with the
    // top of a figure reads before it, then left to right.
    pieces.sort_by_key(|piece| {
        let (x, y, _, height) = piece.rect();
        (
            -((y + height) / 2.0).round() as i32,
            matches!(piece, Piece::Image(_)),
            x.round() as i32,
        )
    });

    let columns = gutters.len() + 1;
    let mut out = Vec::new();
    let mut band: Vec<Vec<Piece<'a>>> = vec![Vec::new(); columns];
    let flush_band = |band: &mut Vec<Vec<Piece<'a>>>, out: &mut Vec<Flow<'a>>| {
        for col in band.iter_mut() {
            let col_lines: Vec<&PdfLine> = col
                .iter()
                .filter_map(|p| match p {
                    Piece::Line(l) => Some(*l),
                    Piece::Image(_) => None,
                })
                .collect();
            let width = col_lines.iter().map(|l| l.width).fold(0.0, f32::max);
            let x = dominant_x(&col_lines);
            // Pieces in a column are already top-down; same-row ties are by x.
            for piece in col.drain(..) {
                out.push(match piece {
                    Piece::Line(line) => Flow::Text(Placed {
                        line,
                        column_width: width,
                        column_x: x,
                        joined: false,
                    }),
                    Piece::Image(image) => Flow::Image {
                        image,
                        page: page_index,
                    },
                });
            }
        }
    };
    for piece in pieces {
        match column_of(&piece) {
            Some(col) => band[col].push(piece),
            None => {
                flush_band(&mut band, &mut out);
                out.push(match piece {
                    Piece::Line(line) => Flow::Text(Placed {
                        line,
                        column_width: page.width,
                        column_x: line.x,
                        joined: false,
                    }),
                    Piece::Image(image) => Flow::Image {
                        image,
                        page: page_index,
                    },
                });
            }
        }
    }
    flush_band(&mut band, &mut out);
    defer_footnotes(out, page, body)
}

/// Small text at the foot of the page (footnotes) reads after the body
/// text of the page, not in the middle of the column it sits under. A
/// page that is mostly small text (references) is left alone.
fn defer_footnotes<'a>(flow: Vec<Flow<'a>>, page: &PdfPage, body: f32) -> Vec<Flow<'a>> {
    let is_note = |f: &Flow| {
        f.text().is_some_and(|p| {
            p.line.font_size < body * 0.85 && p.line.y + p.line.height < page.height * 0.3
        })
    };
    let chars = |f: &Flow| f.text().map_or(0, |p| p.line.text.chars().count());
    let note_chars: usize = flow.iter().filter(|f| is_note(f)).map(chars).sum();
    let total: usize = flow.iter().map(chars).sum();
    if note_chars == 0 || note_chars * 5 >= total * 2 {
        return flow;
    }
    let (notes, mut main): (Vec<_>, Vec<_>) = flow.into_iter().partition(|f| is_note(f));
    main.extend(notes);
    main
}

/// Figures that sit in the middle of a paragraph (at the top of the next
/// column or page) move to where the paragraph ends, so it reads whole.
/// A figure followed by its caption stays put.
fn keep_paragraphs_whole(mut flow: Vec<Flow<'_>>, body: f32) -> Vec<Flow<'_>> {
    let mut held_images: HashSet<usize> = HashSet::new();
    let mut i = 0;
    while i < flow.len() {
        if flow[i].text().is_some() {
            i += 1;
            continue;
        }
        let end = (i..flow.len())
            .find(|&k| flow[k].text().is_some())
            .unwrap_or(flow.len());
        let before = i.checked_sub(1).and_then(|k| flow[k].text());
        let after = flow.get(end).and_then(Flow::text);
        if let (Some(a), Some(b)) = (before, after)
            && !is_caption(&b.line.text)
            && heading_size(b.line, body).is_none()
            && !breaks_by_shape(a, b, body)
        {
            held_images.extend(i..end);
            if let Flow::Text(p) = &mut flow[end] {
                p.joined = true;
            }
        }
        i = end;
    }

    let mut out: Vec<Flow> = Vec::with_capacity(flow.len());
    let mut held: Vec<Flow> = Vec::new();
    for (k, item) in flow.into_iter().enumerate() {
        if held_images.contains(&k) {
            held.push(item);
            continue;
        }
        if let Flow::Text(p) = &item
            && !held.is_empty()
        {
            let prev = out.iter().rev().find_map(Flow::text);
            let ends = heading_size(p.line, body).is_some()
                || prev.is_none_or(|a| breaks_paragraph(a, p, body));
            if ends {
                out.append(&mut held);
            }
        } else {
            out.append(&mut held);
        }
        out.push(item);
    }
    out.append(&mut held);
    out
}

/// Horizontal ranges in the middle of the page that (almost) no narrow
/// line crosses, separating columns with enough text on each side. Figures
/// count toward how much of the page a column fills.
fn find_gutters(
    lines: &[&PdfLine],
    images: &[&PdfImage],
    page: &PdfPage,
    body: f32,
) -> Vec<(f32, f32)> {
    let width = page.width.max(1.0);
    // Headings and titles are often centred across columns, and small type
    // (labels, notes) sits anywhere; only body-sized lines narrower than the
    // page say where the columns are.
    let narrow: Vec<&PdfLine> = lines
        .iter()
        .copied()
        .filter(|l| {
            l.width < width * 0.6
                && l.font_size < body * HEADING_SCALE
                && l.font_size >= body * 0.8
        })
        .collect();
    if narrow.len() < 6 {
        return Vec::new();
    }
    let cells = width.ceil() as usize + 1;
    let mut coverage = vec![0usize; cells];
    for line in &narrow {
        let from = line.x.max(0.0) as usize;
        let to = ((line.x + line.width).min(width) as usize).min(cells - 1);
        for cell in coverage.iter_mut().take(to + 1).skip(from) {
            *cell += 1;
        }
    }
    // A stray centred line (an author, a date) may cross the gutter.
    let allowed = (narrow.len() / 20).max(1);
    let min_gap = (body * 0.5).max(3.0) as usize;
    let lo = (width * 0.2) as usize;
    let hi = (width * 0.8) as usize;
    let mut gutters = Vec::new();
    let mut start: Option<usize> = None;
    for (x, &count) in coverage.iter().enumerate().take(hi + 1).skip(lo) {
        let open = count <= allowed;
        match (open, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                if x - s >= min_gap {
                    gutters.push((s as f32, x as f32));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start
        && hi - s >= min_gap
    {
        gutters.push((s as f32, hi as f32));
    }
    // Every column must hold real text, or this is not a column layout.
    let mut edges = vec![0.0];
    for &(gl, gr) in &gutters {
        edges.push((gl + gr) / 2.0);
    }
    edges.push(width);
    let span_needed = page.height * 0.25;
    for pair in edges.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        let in_col: Vec<&&PdfLine> = narrow
            .iter()
            .filter(|l| {
                let c = l.x + l.width / 2.0;
                c >= left && c < right
            })
            .collect();
        if in_col.len() < 3 {
            return Vec::new();
        }
        let figures = images.iter().filter(|i| {
            let c = i.x + i.width / 2.0;
            c >= left && c < right && i.width < width * 0.6
        });
        let boxes = in_col
            .iter()
            .map(|l| (l.y, l.y + l.height))
            .chain(figures.map(|i| (i.y, i.y + i.height)));
        let (bottom, top) = boxes.fold((f32::MAX, f32::MIN), |(b, t), (lo, hi)| {
            (b.min(lo), t.max(hi))
        });
        if top - bottom < span_needed {
            return Vec::new();
        }
    }
    gutters
}

enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Image {
        page: usize,
        index: u32,
        width: u32,
        height: u32,
        caption: String,
    },
}

impl Block {
    fn html(&self) -> String {
        match self {
            Block::Heading { level, text } => {
                format!("<h{level}>{}</h{level}>", escape_html(text))
            }
            Block::Paragraph(text) => format!("<p>{}</p>", escape_html(text)),
            Block::Image {
                page,
                index,
                width,
                height,
                caption,
            } => {
                let caption = if caption.is_empty() {
                    String::new()
                } else {
                    format!("<figcaption>{}</figcaption>", escape_html(caption))
                };
                format!(
                    "<figure><img src=\"{page}/{index}\" width=\"{width}\" height=\"{height}\" alt=\"\">{caption}</figure>"
                )
            }
        }
    }
}

/// A "Figure N" paragraph right after a figure becomes its caption.
fn attach_captions(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Block::Paragraph(text) = &block
            && is_caption(text)
            && let Some(Block::Image { caption, .. }) = out.last_mut()
            && caption.is_empty()
        {
            *caption = text.clone();
            continue;
        }
        out.push(block);
    }
    out
}

/// Whether a line is set as a heading: larger than the body, or bold at
/// (roughly) body size. Returns the size in half points.
fn heading_size(line: &PdfLine, body: f32) -> Option<u32> {
    let larger = line.font_size >= body * HEADING_SCALE;
    let bold = line.bold && line.font_size >= body * 0.9;
    (larger || bold).then(|| (line.font_size * 2.0).round() as u32)
}

/// Sizes of the larger-than-body headings in the whole document, largest
/// first; they rank into heading levels.
fn heading_sizes(flow: &[Flow<'_>], body: f32) -> Vec<u32> {
    let mut sizes: Vec<u32> = flow
        .iter()
        .filter_map(Flow::text)
        .filter(|p| p.line.text.chars().any(char::is_alphabetic))
        .filter_map(|p| heading_size(p.line, body))
        .filter(|&size| size as f32 / 2.0 >= body * HEADING_SCALE)
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes.dedup();
    sizes
}

/// Consecutive lines that read together: a heading of one size and
/// weight, or body text.
enum Run<'a> {
    Heading { size: u32, lines: Vec<&'a Placed<'a>> },
    Body(Vec<&'a Placed<'a>>),
}

fn push_body<'a>(runs: &mut Vec<Run<'a>>, lines: impl IntoIterator<Item = &'a Placed<'a>>) {
    if let Some(Run::Body(b)) = runs.last_mut() {
        b.extend(lines);
    } else {
        runs.push(Run::Body(lines.into_iter().collect()));
    }
}

/// Blocks for the whole flow: text between figures assembles into headings
/// and paragraphs, figures become image blocks.
fn assemble(flow: &[Flow<'_>], body: f32, sizes: &[u32]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text: Vec<&Placed> = Vec::new();
    for item in flow {
        match item {
            Flow::Text(p) => text.push(p),
            Flow::Image { image, page } => {
                assemble_text(&text, body, sizes, &mut blocks);
                text.clear();
                blocks.push(Block::Image {
                    page: *page,
                    index: image.index,
                    width: image.pixel_width,
                    height: image.pixel_height,
                    caption: String::new(),
                });
            }
        }
    }
    assemble_text(&text, body, sizes, &mut blocks);
    blocks
}

fn assemble_text(placed: &[&Placed<'_>], body: f32, sizes: &[u32], blocks: &mut Vec<Block>) {
    let large = |size: u32| size as f32 / 2.0 >= body * HEADING_SCALE;
    let mut runs: Vec<Run> = Vec::new();
    for (i, &p) in placed.iter().enumerate() {
        let mut candidate = heading_size(p.line, body);
        if let Some(size) = candidate
            && !large(size)
        {
            // Bold at body size is a heading only when it stands apart from
            // the paragraphs around it: not a short last line made of a bold
            // term, and not a first line with a run-in bold lead that the
            // next line continues.
            let continues_previous = i > 0
                && heading_size(placed[i - 1].line, body).is_none()
                && !breaks_paragraph(placed[i - 1], p, body);
            let runs_into_next = placed.get(i + 1).is_some_and(|next| {
                heading_size(next.line, body).is_none() && !breaks_paragraph(p, next, body)
            });
            if continues_previous || runs_into_next {
                candidate = None;
            }
        }
        // Bare numbers (page numbers in a contents list) are never headings.
        if !p.line.text.chars().any(char::is_alphabetic) {
            candidate = None;
        }
        match candidate {
            Some(size) => {
                if let Some(Run::Heading { size: s, lines }) = runs.last_mut()
                    && *s == size
                    && lines.last().is_some_and(|l| l.line.bold == p.line.bold)
                {
                    lines.push(p);
                } else {
                    runs.push(Run::Heading {
                        size,
                        lines: vec![p],
                    });
                }
            }
            None => push_body(&mut runs, [p]),
        }
    }

    // Bold text at body size is a heading only when short; otherwise it is
    // a bold paragraph and reads as body text.
    let mut flat: Vec<Run> = Vec::new();
    for run in runs {
        match run {
            Run::Heading { size, lines }
                if !large(size)
                    && (lines.len() > 2
                        || lines.iter().map(|p| p.line.text.chars().count()).sum::<usize>()
                            > BOLD_HEADING_CHARS) =>
            {
                push_body(&mut flat, lines);
            }
            Run::Body(lines) => push_body(&mut flat, lines),
            heading => flat.push(heading),
        }
    }

    // Rank heading sizes: largest is h2, then h3, everything else h4;
    // bold headings at body size rank below every larger one.
    let level_of = |size: u32| -> u8 {
        let rank = if large(size) {
            sizes.iter().position(|&s| s == size).unwrap_or(sizes.len())
        } else {
            sizes.len()
        };
        (2 + rank.min(2)) as u8
    };

    for run in &flat {
        match run {
            Run::Heading { size, lines } => {
                let mut text = String::new();
                for p in lines {
                    append_line(&mut text, &p.line.text);
                }
                blocks.push(Block::Heading {
                    level: level_of(*size),
                    text,
                });
            }
            Run::Body(lines) => paragraphs(lines, body, blocks),
        }
    }
}

fn paragraphs(lines: &[&Placed<'_>], body: f32, blocks: &mut Vec<Block>) {
    let mut current = String::new();
    let mut prev: Option<&Placed> = None;
    for p in lines {
        if let Some(a) = prev
            && breaks_paragraph(a, p, body)
        {
            flush(&mut current, blocks);
        }
        append_line(&mut current, &p.line.text);
        prev = Some(p);
    }
    flush(&mut current, blocks);
}

/// Whether `b` starts a new paragraph after `a`.
fn breaks_paragraph(a: &Placed<'_>, b: &Placed<'_>, body: f32) -> bool {
    let (la, lb) = (a.line, b.line);
    let line_height = la.height.max(lb.height).max(body);
    // Same page and column, further down: a wide gap is a paragraph space.
    if !b.joined && lb.y < la.y && la.y - lb.y > line_height * 1.5 {
        return true;
    }
    breaks_by_shape(a, b, body)
}

/// Paragraph breaks that the lines themselves show, wherever they sit:
/// a change of size or measure, an indent, a short last line.
fn breaks_by_shape(a: &Placed<'_>, b: &Placed<'_>, body: f32) -> bool {
    let (la, lb) = (a.line, b.line);
    if (la.font_size - lb.font_size).abs() > 0.5 {
        return true;
    }
    // Moving between a column and full-width text starts a new block.
    if (a.column_width - b.column_width).abs() > body {
        return true;
    }
    // A first-line indent: `b` steps in from where the column's lines
    // start. A hanging indent (references): `b` steps back out to the
    // left of where most lines start.
    let step = lb.x - la.x;
    let indent = lb.x - b.column_x;
    if step > body * 0.8 && step < a.column_width * 0.5 && indent > body * 0.4 {
        return true;
    }
    if step < -body * 0.8 && indent < -body * 0.4 {
        return true;
    }
    // A short line ends its paragraph.
    la.width < a.column_width * SHORT_LINE
}

fn append_line(current: &mut String, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let last = current.chars().last();
    let first = line.chars().next();
    match (last, first) {
        (None, _) => {}
        (Some('-'), Some(c)) if c.is_lowercase() => {
            current.pop();
        }
        (Some(a), Some(b)) if is_cjk(a) || is_cjk(b) => {}
        _ => current.push(' '),
    }
    current.push_str(line);
}

fn flush(current: &mut String, blocks: &mut Vec<Block>) {
    if !current.trim().is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(current)));
    }
    current.clear();
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eddy_model::BlockKind;

    const W: f32 = 612.0;
    const H: f32 = 792.0;

    /// A body line whose width is about 5 points per character.
    fn line(text: &str, x: f32, y: f32, size: f32) -> PdfLine {
        PdfLine {
            text: text.into(),
            x,
            y,
            width: text.chars().count() as f32 * size * 0.5,
            height: size * 1.15,
            font_size: size,
            bold: false,
        }
    }

    fn wide(text: &str, x: f32, y: f32, width: f32, size: f32) -> PdfLine {
        PdfLine {
            width,
            ..line(text, x, y, size)
        }
    }

    fn bold(mut l: PdfLine) -> PdfLine {
        l.bold = true;
        l
    }

    fn page(lines: Vec<PdfLine>) -> PdfPage {
        PdfPage {
            width: W,
            height: H,
            lines,
            images: Vec::new(),
        }
    }

    fn document(pages: &[PdfPage]) -> Document {
        super::document(pages, "key")
    }

    /// Fills a column: `n` full-width lines of 10 pt text from `top` down.
    fn column(x: f32, width: f32, top: f32, n: usize, label: &str) -> Vec<PdfLine> {
        (0..n)
            .map(|i| wide(&format!("{label} line {i} of the column text"), x, top - 12.0 * i as f32, width, 10.0))
            .collect()
    }

    fn texts(doc: &Document) -> Vec<String> {
        doc.blocks.iter().map(|b| b.plain_text()).collect()
    }

    #[test]
    fn running_headers_and_page_numbers_go_but_single_mentions_stay() {
        let pages: Vec<PdfPage> = (1..=3)
            .map(|n| {
                let mut lines = vec![
                    line("Journal of Things", 72.0, 760.0, 9.0),
                    line(&n.to_string(), 300.0, 30.0, 9.0),
                    line(&format!("- {n} -"), 500.0, 30.0, 9.0),
                ];
                lines.extend(column(72.0, 468.0, 700.0, 5, &format!("p{n}")));
                if n == 2 {
                    lines.push(line("Only on this page", 72.0, 770.0, 9.0));
                }
                page(lines)
            })
            .collect();
        let doc = document(&pages);
        let all = texts(&doc).join("\n");
        assert!(!all.contains("Journal of Things"), "{all}");
        assert!(!all.contains("- 2 -"), "{all}");
        assert!(!all.contains("\n2\n"), "{all}");
        assert!(all.contains("Only on this page"), "{all}");
        assert!(all.contains("p1 line 0"), "{all}");
    }

    #[test]
    fn page_number_shapes() {
        for s in ["12", "- 12 -", "Page 12", "12 of 30", "12 / 30", "第 12 页", "xiv", "p. 7"] {
            assert!(is_page_number(s), "{s}");
        }
        for s in ["Chapter 12", "12 apples", "1.2", "", "Page"] {
            assert!(!is_page_number(s), "{s}");
        }
    }

    #[test]
    fn two_columns_read_left_then_right_with_spanning_title() {
        let mut lines = vec![
            bold(wide("A Spanning Title", 150.0, 720.0, 312.0, 18.0)),
            // A centred author line at body size crosses the gutter too.
            line("Ada Lovelace", 270.0, 700.0, 10.0),
        ];
        lines.extend(column(72.0, 220.0, 680.0, 20, "left"));
        lines.extend(column(320.0, 220.0, 680.0, 20, "right"));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert_eq!(t[0], "A Spanning Title");
        assert!(matches!(doc.blocks[0].kind, BlockKind::Heading { level: 2, .. }));
        assert_eq!(t[1], "Ada Lovelace");
        // The text flows from the bottom of the left column into the right.
        assert_eq!(t.len(), 3, "{t:?}");
        let body = &t[2];
        let pos = |s: &str| body.find(s).unwrap_or_else(|| panic!("{s} in {body}"));
        assert!(body.starts_with("left line 0"));
        assert!(pos("left line 19") < pos("right line 0"));
        assert!(body.ends_with("right line 19 of the column text"));
    }

    #[test]
    fn a_wide_caption_splits_columns_into_bands() {
        let mut lines = column(72.0, 220.0, 700.0, 8, "topleft");
        lines.extend(column(320.0, 220.0, 700.0, 8, "topright"));
        lines.push(wide("Figure 1: something wide across the page.", 72.0, 580.0, 468.0, 10.0));
        lines.extend(column(72.0, 220.0, 560.0, 8, "botleft"));
        lines.extend(column(320.0, 220.0, 560.0, 8, "botright"));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        let all = t.join("\n");
        let pos = |s: &str| all.find(s).unwrap_or_else(|| panic!("{s} in {all}"));
        assert!(pos("topleft line 0") < pos("topright line 0"));
        assert!(pos("topright line 7") < pos("Figure 1"));
        assert!(pos("Figure 1") < pos("botleft line 0"));
        assert!(pos("botleft line 7") < pos("botright line 0"));
        assert!(t.iter().any(|p| p.starts_with("Figure 1")), "caption is its own block: {t:?}");
    }

    #[test]
    fn footnotes_under_a_column_read_after_the_page() {
        let mut lines = column(72.0, 220.0, 700.0, 20, "left");
        lines.extend(column(320.0, 220.0, 700.0, 20, "right"));
        lines.push(wide("1 A footnote in small type under the left column.", 72.0, 80.0, 200.0, 8.0));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert_eq!(t.len(), 2, "{t:?}");
        assert!(t[0].contains("left line 19 of the column text right line 0"), "{}", t[0]);
        assert!(t[1].starts_with("1 A footnote"));
    }

    #[test]
    fn single_column_text_is_not_split_by_an_indent() {
        let mut lines = vec![wide("First paragraph starts here.", 92.0, 700.0, 440.0, 10.0)];
        lines.extend(column(72.0, 468.0, 688.0, 10, "body"));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert_eq!(t.len(), 1, "{t:?}");
        assert!(t[0].starts_with("First paragraph"));
    }

    #[test]
    fn headings_rank_by_size_and_bold_short_lines_count() {
        let lines = vec![
            bold(line("Big", 72.0, 740.0, 18.0)),
            line("Body text after the big heading runs on.", 72.0, 720.0, 10.0),
            bold(line("Medium", 72.0, 700.0, 14.0)),
            line("More body text after the medium heading.", 72.0, 680.0, 10.0),
            bold(line("Bold at body size", 72.0, 660.0, 10.0)),
            line("Body again.", 72.0, 640.0, 10.0),
            bold(wide(
                "A bold paragraph that is far too long to be a heading because it runs on and on and on and on and keeps going past the limit set.",
                72.0, 620.0, 468.0, 10.0,
            )),
            bold(wide("and continues on a second bold line of the same size and weight here", 72.0, 608.0, 468.0, 10.0)),
            bold(wide("and a third bold line to make it clearly a paragraph not a heading", 72.0, 596.0, 468.0, 10.0)),
        ];
        // Pad with body text so 10 pt is the body size.
        let mut all = lines;
        all.extend(column(72.0, 468.0, 500.0, 10, "pad"));
        let doc = document(&[page(all)]);
        let levels: Vec<(String, Option<u8>)> = doc
            .blocks
            .iter()
            .map(|b| {
                let level = match b.kind {
                    BlockKind::Heading { level, .. } => Some(level),
                    _ => None,
                };
                (b.plain_text(), level)
            })
            .collect();
        assert_eq!(levels[0], ("Big".to_string(), Some(2)));
        assert_eq!(levels[2], ("Medium".to_string(), Some(3)));
        assert_eq!(levels[4], ("Bold at body size".to_string(), Some(4)));
        assert!(levels[6].0.starts_with("A bold paragraph"));
        assert_eq!(levels[6].1, None);
    }

    #[test]
    fn a_bold_term_ending_a_paragraph_is_not_a_heading() {
        let mut lines = vec![
            wide("The variable that holds the last read line is called", 72.0, 700.0, 468.0, 10.0),
            bold(line("REPLY.", 72.0, 688.0, 10.0)),
            bold(line("4.2.2. New and Modified Builtins", 72.0, 668.0, 10.0)),
            wide("Several builtins were added to the shell in this release.", 72.0, 652.0, 468.0, 10.0),
        ];
        lines.extend(column(72.0, 468.0, 600.0, 10, "pad"));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert!(t[0].ends_with("is called REPLY."), "{}", t[0]);
        assert_eq!(t[1], "4.2.2. New and Modified Builtins");
        assert!(matches!(doc.blocks[1].kind, BlockKind::Heading { .. }));
    }

    #[test]
    fn a_run_in_bold_lead_stays_with_its_paragraph() {
        let mut lines = vec![
            line("Here we describe the evaluations:", 72.0, 700.0, 10.0),
            bold(wide("Responsible Scaling Policy evaluations. The model advances the frontier and", 72.0, 680.0, 468.0, 10.0)),
            wide("is the most capable one we have trained, so we tested its risk level.", 72.0, 668.0, 468.0, 10.0),
            bold(line("Cyber", 72.0, 648.0, 10.0)),
            wide("A real heading above stands apart from this text by a gap.", 72.0, 630.0, 468.0, 10.0),
            bold(line("12", 72.0, 610.0, 10.0)),
        ];
        lines.extend(column(72.0, 468.0, 590.0, 10, "pad"));
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert!(t[1].starts_with("Responsible Scaling") && t[1].ends_with("risk level."), "{}", t[1]);
        assert!(matches!(doc.blocks[1].kind, BlockKind::Paragraph { .. }));
        assert_eq!(t[2], "Cyber");
        assert!(matches!(doc.blocks[2].kind, BlockKind::Heading { .. }));
        assert_eq!(t[4], "12");
        assert!(matches!(doc.blocks[4].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn paragraphs_break_on_gaps_indents_and_short_lines_and_join_hyphens() {
        let lines = vec![
            wide("Importance sampling draws more samples where the inte-", 72.0, 700.0, 468.0, 10.0),
            wide("grand is large and so reduces the variance of Monte", 72.0, 688.0, 468.0, 10.0),
            line("Carlo estimates.", 72.0, 676.0, 10.0),
            wide("A new paragraph after a short line fills the width out", 72.0, 664.0, 468.0, 10.0),
            wide("and then there is a vertical gap before the next one.", 72.0, 652.0, 468.0, 10.0),
            wide("After the gap the text continues at full width again", 72.0, 630.0, 468.0, 10.0),
            wide("and the following line is indented like a first line.", 72.0, 618.0, 468.0, 10.0),
            wide("Indented first line of the last paragraph on the page", 92.0, 606.0, 448.0, 10.0),
            wide("which continues at the margin in the normal fashion.", 72.0, 594.0, 468.0, 10.0),
        ];
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert_eq!(
            t,
            [
                "Importance sampling draws more samples where the integrand is large and so reduces the variance of Monte Carlo estimates.",
                "A new paragraph after a short line fills the width out and then there is a vertical gap before the next one.",
                "After the gap the text continues at full width again and the following line is indented like a first line.",
                "Indented first line of the last paragraph on the page which continues at the margin in the normal fashion.",
            ]
        );
    }

    #[test]
    fn hanging_indents_start_entries_at_the_outdented_line() {
        let lines = vec![
            wide("Ada Lovelace. Notes on the analytical engine. In Scientific", 72.0, 700.0, 468.0, 10.0),
            wide("Memoirs, volume 3, pages 666-731, 1843.", 90.0, 688.0, 250.0, 10.0),
            wide("Charles Babbage. On the mathematical powers of the calculating", 72.0, 676.0, 468.0, 10.0),
            wide("engine. Unpublished manuscript, 1837. Reprinted in the", 90.0, 664.0, 450.0, 10.0),
            wide("collected works, volume 2.", 90.0, 652.0, 120.0, 10.0),
            wide("Grace Hopper. The education of a computer. In Proceedings", 72.0, 640.0, 468.0, 10.0),
            wide("of the ACM National Meeting, pages 243-249, 1952.", 90.0, 628.0, 300.0, 10.0),
        ];
        let doc = document(&[page(lines)]);
        let t = texts(&doc);
        assert_eq!(t.len(), 3, "{t:?}");
        assert!(t[0].starts_with("Ada Lovelace") && t[0].ends_with("1843."), "{}", t[0]);
        assert!(t[1].starts_with("Charles Babbage") && t[1].ends_with("volume 2."), "{}", t[1]);
        assert!(t[2].starts_with("Grace Hopper"), "{}", t[2]);
    }

    #[test]
    fn cjk_lines_join_without_spaces_and_paragraphs_cross_pages() {
        let first = page(vec![
            wide("第二页的内容比较长一些，这一行写满了整个版面的宽度所以", 72.0, 100.0, 468.0, 10.0),
        ]);
        let second = page(vec![wide("继续到下一页，然后结束。", 72.0, 700.0, 468.0, 10.0)]);
        let doc = document(&[first, second]);
        assert_eq!(
            texts(&doc),
            ["第二页的内容比较长一些，这一行写满了整个版面的宽度所以继续到下一页，然后结束。"]
        );
    }

    fn image(index: u32, x: f32, y: f32, width: f32, height: f32, fingerprint: u64) -> PdfImage {
        PdfImage {
            index,
            x,
            y,
            width,
            height,
            pixel_width: (width * 3.0) as u32,
            pixel_height: (height * 3.0) as u32,
            fingerprint,
        }
    }

    fn images(doc: &Document) -> Vec<(String, String)> {
        doc.blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Image { src, .. } => Some((src.clone(), b.plain_text())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_figure_sits_between_paragraphs_with_its_caption() {
        let mut p = page(vec![
            line("Text before the figure ends here.", 72.0, 700.0, 10.0),
            line("Figure 1: Sampling error by method.", 72.0, 470.0, 9.0),
            line("Text after the figure starts here.", 72.0, 440.0, 10.0),
            line("16 GB", 100.0, 550.0, 8.0),
        ]);
        p.lines.extend(column(72.0, 468.0, 400.0, 10, "pad"));
        p.images.push(image(5, 72.0, 490.0, 300.0, 190.0, 1));
        let doc = document(&[p]);
        let t = texts(&doc);
        assert_eq!(t[0], "Text before the figure ends here.");
        assert!(matches!(doc.blocks[1].kind, BlockKind::Image { .. }), "{t:?}");
        assert_eq!(
            images(&doc),
            [("pdf://key/0/5".to_string(), "Figure 1: Sampling error by method.".to_string())]
        );
        assert_eq!(t[2], "Text after the figure starts here.");
        assert!(!t.iter().any(|s| s.contains("16 GB")), "labels on the figure go: {t:?}");
    }

    #[test]
    fn covers_backgrounds_watermarks_logos_and_decorations_are_left_out() {
        let body = |label: &str| column(72.0, 468.0, 650.0, 14, label);
        let mut cover = page(vec![line("A Title", 72.0, 700.0, 24.0)]);
        cover.images.push(image(1, 50.0, 200.0, 500.0, 400.0, 10));
        cover.images.push(image(2, 500.0, 740.0, 60.0, 30.0, 99));
        let mut second = page(body("second"));
        second.images.push(image(1, 0.0, 0.0, W, H, 11));
        second.images.push(image(2, 500.0, 740.0, 60.0, 30.0, 99));
        second.images.push(image(3, 72.0, 450.0, 400.0, 200.0, 12));
        second.images.push(image(4, 72.0, 300.0, 20.0, 20.0, 13));
        second.images.push(image(5, 72.0, 20.0, 200.0, 40.0, 14));
        second.images.push(image(6, 72.0, 380.0, 468.0, 2.0, 15));
        let mut third = page(body("third"));
        third.images.push(image(2, 500.0, 740.0, 60.0, 30.0, 99));
        third.images.push(image(7, 100.0, 100.0, 300.0, 200.0, 16));
        let doc = document(&[cover, second, third]);
        let kept: Vec<String> = images(&doc).into_iter().map(|(src, _)| src).collect();
        // The watermark under the body text of page 2, the cover picture,
        // the full-page background, the logo on every page, the icon, the
        // footer strip and the rule all go; the figure on page 3 stays.
        assert_eq!(kept, ["pdf://key/2/7"]);
    }

    #[test]
    fn a_figure_at_the_top_of_a_column_waits_for_the_paragraph_to_end() {
        let mut lines = column(72.0, 220.0, 700.0, 20, "left");
        lines.extend(column(320.0, 220.0, 520.0, 5, "right"));
        let mut p = page(lines);
        p.images.push(image(3, 320.0, 540.0, 220.0, 170.0, 1));
        let doc = document(&[p]);
        let t = texts(&doc);
        assert!(
            t[0].contains("left line 19 of the column text right line 0"),
            "the paragraph runs on past the figure: {t:?}"
        );
        assert!(matches!(doc.blocks[1].kind, BlockKind::Image { .. }), "{t:?}");
    }

    #[test]
    fn caption_shapes() {
        for s in ["Figure 3: x", "Fig. 3", "FIGURE 2.", "Figure A1", "[Figure 2.2.4.1.A] Evals", "图 3 示意", "图3"] {
            assert!(is_caption(s), "{s}");
        }
        for s in ["Figures show", "Figure out how", "Fig tree", "图书馆"] {
            assert!(!is_caption(s), "{s}");
        }
    }

    #[test]
    fn empty_input_gives_an_empty_document() {
        assert!(document(&[]).blocks.is_empty());
        assert!(document(&[page(vec![])]).blocks.is_empty());
    }
}
