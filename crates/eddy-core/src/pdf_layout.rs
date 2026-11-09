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
use eddy_model::{BlockKind, Document, PDF_SCHEME};

use crate::import::{PdfImage, PdfLine, PdfPage, PdfRun};
use crate::pdf_listing::{self, Listing};
use crate::pdf_table::{self, Table};

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
    let blocks = reflow(pages);
    let html: String = blocks.iter().map(Block::html).collect();
    let base = format!("{PDF_SCHEME}://{key}/");
    let mut doc = html_to_document(
        &html,
        &ConvertOptions {
            base_url: Some(&base),
            title: None,
        },
    );
    doc.pages = page_starts(&blocks, &doc.blocks, pages.len());
    doc
}

/// The block each page starts at. Conversion keeps the blocks in order but
/// may split one (a paragraph with display TeX in it), so converted blocks
/// are matched to the blocks they came from by their text.
fn page_starts(blocks: &[Block], converted: &[eddy_model::Block], pages: usize) -> Vec<String> {
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let texts: Vec<String> = blocks.iter().map(|b| squash(&b.text())).collect();
    // Whether converted block `c` is where block `i` starts.
    let starts = |i: usize, c: &eddy_model::Block| match (&blocks[i], &c.kind) {
        (Block::Image { .. }, BlockKind::Image { .. }) => true,
        (Block::Image { .. }, _) | (_, BlockKind::Image { .. }) => false,
        _ => {
            let head: String = squash(&c.plain_text()).chars().take(24).collect();
            !head.is_empty() && texts[i].starts_with(&head)
        }
    };
    // Page of each converted block.
    let mut placed: Vec<(usize, &str)> = Vec::with_capacity(converted.len());
    let mut j = 0;
    for i in 0..blocks.len() {
        let Some(c) = converted.get(j) else { break };
        // Dropped in conversion: its converted block is the next one's.
        if !starts(i, c) && i + 1 < blocks.len() && starts(i + 1, c) {
            continue;
        }
        placed.push((blocks[i].page(), &c.id));
        j += 1;
        // The rest of a block conversion split.
        while let Some(c) = converted.get(j) {
            let part = !matches!(blocks[i], Block::Image { .. })
                && !matches!(c.kind, BlockKind::Image { .. })
                && texts[i].contains(&squash(&c.plain_text()))
                && (i + 1 >= blocks.len() || !starts(i + 1, c));
            if !part {
                break;
            }
            placed.push((blocks[i].page(), &c.id));
            j += 1;
        }
    }
    if placed.is_empty() {
        return Vec::new();
    }
    let last = placed[placed.len() - 1].1;
    (0..pages)
        .map(|page| {
            placed
                .iter()
                .find(|(p, _)| *p >= page)
                .map_or(last, |(_, id)| *id)
                .to_string()
        })
        .collect()
}

fn reflow(pages: &[PdfPage]) -> Vec<Block> {
    let body = body_size(pages);
    let label_edge = label_column(pages, body);
    let tables: Vec<Vec<Table>> = pages.iter().map(pdf_table::tables).collect();
    let pages: Vec<PdfPage> = pages
        .iter()
        .zip(&tables)
        .map(|(page, tables)| split_lines(page, tables, body, label_edge))
        .collect();
    let pages = pages.as_slice();
    let listings: Vec<Vec<Listing>> = pages
        .iter()
        .zip(&tables)
        .map(|(page, tables)| pdf_listing::listings(page, |l| tables.iter().any(|t| t.holds(l))))
        .collect();
    let repeated = repeated_margin_text(pages);
    let repeated_images = repeated_images(pages);
    let slide_frames = slide_frames(pages);
    let mut flow = Vec::new();
    for (n, page) in pages.iter().enumerate() {
        let slides = page_slides(page, &slide_frames);
        let tables: Vec<&Table> = tables[n]
            .iter()
            .filter(|t| !slides.iter().any(|s| t.center_inside(s)))
            .collect();
        let listings: Vec<&Listing> = listings[n]
            .iter()
            .filter(|b| !slides.iter().any(|s| b.center_inside(s)))
            .collect();
        let lines: Vec<&PdfLine> = page
            .lines
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .filter(|l| !is_margin_junk(l, page, &repeated))
            .filter(|l| !slides.iter().any(|s| center_inside(l, s)))
            .filter(|l| !tables.iter().any(|t| t.holds(l)))
            .filter(|l| !listings.iter().any(|b| b.holds(l)))
            .collect();
        // A slide shows whole, as it was designed; what is drawn on it
        // (its text, pictures, diagrams) is part of that picture.
        let images: Vec<&PdfImage> = page
            .images
            .iter()
            .filter(|i| !slides.iter().any(|s| image_inside(i, s)))
            .filter(|i| keeps_image(i, n, page, &lines, body, &repeated_images))
            .chain(slides.iter().copied())
            .collect();
        // Labels drawn as text over a kept figure belong to the figure.
        let lines: Vec<&PdfLine> = lines
            .into_iter()
            .filter(|l| !images.iter().any(|i| is_label_of(l, i)))
            .collect();
        let labels = label_edge.map_or_else(Vec::new, |edge| page_labels(&lines, edge, body));
        let lines: Vec<&PdfLine> = lines
            .into_iter()
            .filter(|l| !labels.iter().flatten().any(|m| std::ptr::eq(*m, *l)))
            .collect();
        let content = PageContent {
            lines: &lines,
            images: &images,
            tables: &tables,
            listings: &listings,
            labels: &labels,
        };
        flow.extend(order_page(&content, &slides, n, page, body));
    }
    let flow = keep_paragraphs_whole(flow, body);
    let sizes = heading_sizes(&flow, body);
    attach_captions(assemble(&flow, body, &sizes))
}

/// Where slides sit on pages of slides printed with their notes (or a few
/// to a page): rectangles of a slide's shape, well short of the whole
/// page, drawn at the same place on most pages. As `(x, y, width, height)`.
fn slide_frames(pages: &[PdfPage]) -> Vec<(f32, f32, f32, f32)> {
    if pages.len() < 3 {
        return Vec::new();
    }
    let key = |f: &PdfImage| {
        let r = |v: f32| (v / 2.0).round() as i32;
        (r(f.x), r(f.y), r(f.width), r(f.height))
    };
    let mut counts: HashMap<(i32, i32, i32, i32), (usize, &PdfImage)> = HashMap::new();
    for page in pages {
        let area = page.width * page.height;
        let mut seen = HashSet::new();
        for frame in &page.frames {
            let shape = frame.width / frame.height.max(1.0);
            if !(1.2..=2.4).contains(&shape) || frame.width * frame.height > area * 0.6 {
                continue;
            }
            if seen.insert(key(frame)) {
                counts.entry(key(frame)).or_insert((0, frame)).0 += 1;
            }
        }
    }
    let mut frames: Vec<&PdfImage> = counts
        .into_values()
        .filter(|&(n, _)| n >= 3 && n * 2 > pages.len())
        .map(|(_, f)| f)
        .collect();
    frames.sort_by(|a, b| (b.width * b.height).total_cmp(&(a.width * a.height)));
    // A slide's background and its border are the same place.
    let mut kept: Vec<&PdfImage> = Vec::new();
    for frame in frames {
        if !kept.iter().any(|k| image_inside(frame, k)) {
            kept.push(frame);
        }
    }
    kept.iter().map(|f| (f.x, f.y, f.width, f.height)).collect()
}

/// The slides on a page: for each slide frame, the largest rectangle drawn
/// there.
fn page_slides<'a>(page: &'a PdfPage, frames: &[(f32, f32, f32, f32)]) -> Vec<&'a PdfImage> {
    let near = |a: f32, b: f32| (a - b).abs() <= 3.0;
    frames
        .iter()
        .filter_map(|&(x, y, width, height)| {
            page.frames
                .iter()
                .filter(|f| {
                    near(f.x, x) && near(f.y, y) && near(f.width, width) && near(f.height, height)
                })
                .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        })
        .collect()
}

fn image_inside(image: &PdfImage, outer: &PdfImage) -> bool {
    let cx = image.x + image.width / 2.0;
    let cy = image.y + image.height / 2.0;
    cx > outer.x && cx < outer.x + outer.width && cy > outer.y && cy < outer.y + outer.height
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

/// Which label a line belongs to: its page and its place among the page's
/// labels.
type LabelId = (usize, usize);

/// A line in reading order with the shape of the column it sits in.
struct Placed<'a> {
    line: &'a PdfLine,
    page: usize,
    column_width: f32,
    /// Where most lines of the column start; a line to the right of it is
    /// indented, one to the left is outdented (a hanging indent).
    column_x: f32,
    /// The paragraph continues into this line across a figure, so the
    /// vertical gap before it is not a paragraph space.
    joined: bool,
    /// The line is (part of) a label in the margin column, which heads the
    /// text set beside it.
    label: Option<LabelId>,
}

/// What reads next: a line of text, a figure or a table.
enum Flow<'a> {
    Text(Placed<'a>),
    /// A slide is where its notes start, never moved to let a paragraph
    /// read on.
    Image {
        image: &'a PdfImage,
        page: usize,
        slide: bool,
    },
    /// Tables stay where they are.
    Table { table: &'a Table, page: usize },
    /// A boxed code listing, which floats like a figure.
    Listing { listing: &'a Listing, page: usize },
    /// Figures set side by side with their own captions under them, read
    /// one after another and moved as one.
    Group(Vec<Flow<'a>>),
}

impl<'a> Flow<'a> {
    fn text(&self) -> Option<&Placed<'a>> {
        match self {
            Flow::Text(p) => Some(p),
            Flow::Image { .. } | Flow::Table { .. } | Flow::Listing { .. } | Flow::Group(_) => None,
        }
    }

    /// Top edge on the page.
    fn top(&self) -> f32 {
        match self {
            Flow::Text(p) => p.line.y + p.line.height,
            Flow::Image { image, .. } => image.y + image.height,
            Flow::Table { table, .. } => table.top,
            Flow::Listing { listing, .. } => listing.top,
            Flow::Group(items) => items.iter().map(Flow::top).fold(f32::MIN, f32::max),
        }
    }

    /// Kept where it is when a paragraph runs past it.
    fn pinned(&self) -> bool {
        match self {
            Flow::Image { slide, .. } => *slide,
            Flow::Table { .. } => true,
            Flow::Group(items) => items.iter().any(Flow::pinned),
            Flow::Text(_) | Flow::Listing { .. } => false,
        }
    }
}

#[derive(Clone, Copy)]
enum Piece<'a> {
    Line(&'a PdfLine),
    Image(&'a PdfImage),
    Table(&'a Table),
    Listing(&'a Listing),
}

impl Piece<'_> {
    /// Left, bottom, width, height.
    fn rect(&self) -> (f32, f32, f32, f32) {
        match self {
            Piece::Line(l) => (l.x, l.y, l.width, l.height),
            Piece::Image(i) => (i.x, i.y, i.width, i.height),
            Piece::Table(t) => (t.left, t.bottom, t.right - t.left, t.top - t.bottom),
            Piece::Listing(b) => (b.left, b.bottom, b.right - b.left, b.top - b.bottom),
        }
    }
}

/// A column's pieces in reading order: one at a time, or a group of
/// figures side by side, each column of the group top-down.
enum Part<'a> {
    One(Piece<'a>),
    Group(Vec<Vec<Piece<'a>>>),
}

/// Finds figures set side by side in a column (sub-figures, listings next
/// to each other), each with its caption lines under it. Without this
/// their captions interleave line by line. Text beside a figure (wrapped
/// around it) is not a group: every column of a group holds a figure or
/// only lines smaller than the body text.
fn side_by_side<'a>(col: Vec<Piece<'a>>, body: f32) -> Vec<Part<'a>> {
    let left = col.iter().map(|p| p.rect().0).fold(f32::MAX, f32::min);
    let right = col.iter().map(|p| p.rect().0 + p.rect().2).fold(f32::MIN, f32::max);
    let narrow = |p: &Piece| p.rect().2 < (right - left) * 0.6;
    let mut out = Vec::with_capacity(col.len());
    let mut i = 0;
    while i < col.len() {
        if !narrow(&col[i]) {
            out.push(Part::One(col[i]));
            i += 1;
            continue;
        }
        let end = (i..col.len()).find(|&k| !narrow(&col[k])).unwrap_or(col.len());
        let run = &col[i..end];
        match group_in(run, body) {
            Some((start, stop, columns)) => {
                out.extend(run[..start].iter().map(|&p| Part::One(p)));
                out.push(Part::Group(columns));
                out.extend(run[stop..].iter().map(|&p| Part::One(p)));
            }
            None => out.extend(run.iter().map(|&p| Part::One(p))),
        }
        i = end;
    }
    out
}

/// Pieces whose horizontal extents touch, as columns left to right; each
/// column is indexes into `pieces`, in their order.
fn x_columns(pieces: &[Piece<'_>], body: f32) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by(|&a, &b| pieces[a].rect().0.total_cmp(&pieces[b].rect().0));
    let mut columns: Vec<(f32, Vec<usize>)> = Vec::new();
    for k in order {
        let (x, _, w, _) = pieces[k].rect();
        match columns.last_mut() {
            Some((right, members)) if x < *right + body * 0.3 => {
                *right = right.max(x + w);
                members.push(k);
            }
            _ => columns.push((x + w, vec![k])),
        }
    }
    columns
        .into_iter()
        .map(|(_, mut members)| {
            members.sort_unstable();
            members
        })
        .collect()
}

/// The stretch of `run` that is a side-by-side group, as its start, end
/// and columns of pieces.
fn group_in<'a>(run: &[Piece<'a>], body: f32) -> Option<(usize, usize, Vec<Vec<Piece<'a>>>)> {
    let is_figure = |p: &Piece| !matches!(p, Piece::Line(_));
    if !run.iter().any(is_figure) {
        return None;
    }
    let columns = x_columns(run, body);
    if columns.len() < 2 {
        return None;
    }
    let mut column_of = vec![0; run.len()];
    for (c, members) in columns.iter().enumerate() {
        for &k in members {
            column_of[k] = c;
        }
    }
    let overlaps = |a: &Piece, b: &Piece| {
        let (_, ay, _, ah) = a.rect();
        let (_, by, _, bh) = b.rect();
        ay < by + bh + 1.0 && by < ay + ah + 1.0
    };
    // Something sits level with it in another column of the group.
    let partnered = |k: usize| (0..run.len()).any(|m| column_of[m] != column_of[k] && overlaps(&run[k], &run[m]));
    let start = (0..run.len()).find(|&k| partnered(k))?;
    let mut stop = (0..run.len()).rev().find(|&k| partnered(k))? + 1;
    // A caption under a figure can run a line or two longer than the one
    // beside it.
    while stop < run.len() {
        let Piece::Line(line) = run[stop] else { break };
        let above = (start..stop).rev().find(|&k| column_of[k] == column_of[stop]);
        let close = above.is_some_and(|k| {
            let (_, y, _, _) = run[k].rect();
            y - (line.y + line.height) < line.height.max(body) * 0.8
        });
        if line.font_size >= body * 0.95 || !close {
            break;
        }
        stop += 1;
    }
    let pieces = &run[start..stop];
    let columns: Vec<Vec<Piece>> = x_columns(pieces, body)
        .into_iter()
        .map(|members| members.into_iter().map(|k| pieces[k]).collect())
        .collect();
    let holds_figure = |c: &Vec<Piece>| {
        c.iter().any(is_figure)
            || c.iter()
                .all(|p| matches!(p, Piece::Line(l) if l.font_size < body * 0.95))
    };
    (columns.len() >= 2 && columns.iter().all(holds_figure)).then_some((start, stop, columns))
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

/// What a page holds once margins, slides and labels are set apart.
struct PageContent<'c, 'a> {
    lines: &'c [&'a PdfLine],
    images: &'c [&'a PdfImage],
    tables: &'c [&'a Table],
    listings: &'c [&'a Listing],
    /// Labels in the margin column, each its lines top-down.
    labels: &'c [Vec<&'a PdfLine>],
}

/// Reading order for one page: columns between gutters, read one after
/// another within each band that spanning lines and images delimit.
/// Labels go before the text they stand beside.
fn order_page<'a>(
    content: &PageContent<'_, 'a>,
    slides: &[&PdfImage],
    page_index: usize,
    page: &PdfPage,
    body: f32,
) -> Vec<Flow<'a>> {
    let PageContent {
        lines,
        images,
        tables,
        listings,
        labels,
    } = *content;
    let gutters = find_gutters(lines, images, page, body);
    let figure = |image: &'a PdfImage| Flow::Image {
        image,
        page: page_index,
        slide: slides.iter().any(|s| std::ptr::eq(*s, image)),
    };
    let column_of = |piece: &Piece| -> Option<usize> {
        let (x, _, width, _) = piece.rect();
        let right = x + width;
        let center = x + width / 2.0;
        // A piece spans when it reaches past a gutter on both sides, or is
        // centred on one (a short title over two columns).
        if gutters
            .iter()
            .any(|&(gl, gr)| (x < gl - body && right > gr + body) || (center >= gl && center <= gr))
        {
            return None;
        }
        Some(gutters.iter().filter(|&&(_, gr)| center > gr).count())
    };

    let mut pieces: Vec<Piece<'a>> = lines
        .iter()
        .map(|l| Piece::Line(l))
        .chain(images.iter().map(|i| Piece::Image(i)))
        .chain(tables.iter().map(|t| Piece::Table(t)))
        .chain(listings.iter().map(|b| Piece::Listing(b)))
        .collect();
    // Top-down by 2-point rows; within a row, a title set flush with the
    // top of a figure reads before it, then left to right.
    pieces.sort_by_key(|piece| {
        let (x, y, _, height) = piece.rect();
        (
            -((y + height) / 2.0).round() as i32,
            !matches!(piece, Piece::Line(_)),
            x.round() as i32,
        )
    });

    let placed = |line: &'a PdfLine, column_width: f32, column_x: f32| {
        Flow::Text(Placed {
            line,
            page: page_index,
            column_width,
            column_x,
            joined: false,
            label: None,
        })
    };
    // Lines across the columns are measured against the widest of them,
    // the page's full measure.
    let spanning_width = pieces
        .iter()
        .filter(|p| column_of(p).is_none())
        .filter_map(|p| match p {
            Piece::Line(l) => Some(l.width),
            _ => None,
        })
        .fold(0.0, f32::max);
    let columns = gutters.len() + 1;
    let mut out = Vec::new();
    let mut band: Vec<Vec<Piece<'a>>> = vec![Vec::new(); columns];
    // The measure of a column: its widest line and where most lines start.
    let measure = |pieces: &[Piece<'a>]| {
        let lines: Vec<&PdfLine> = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Line(l) => Some(*l),
                Piece::Image(_) | Piece::Table(_) | Piece::Listing(_) => None,
            })
            .collect();
        (lines.iter().map(|l| l.width).fold(0.0, f32::max), dominant_x(&lines))
    };
    let to_flow = |piece: Piece<'a>, width: f32, x: f32| match piece {
        Piece::Line(line) => placed(line, width, x),
        Piece::Image(image) => figure(image),
        Piece::Table(table) => Flow::Table {
            table,
            page: page_index,
        },
        Piece::Listing(listing) => Flow::Listing {
            listing,
            page: page_index,
        },
    };
    let flush_band = |band: &mut Vec<Vec<Piece<'a>>>, out: &mut Vec<Flow<'a>>| {
        for col in band.iter_mut() {
            let pieces = std::mem::take(col);
            let (width, x) = measure(&pieces);
            // Pieces in a column are already top-down; same-row ties are by x.
            for part in side_by_side(pieces, body) {
                match part {
                    Part::One(piece) => out.push(to_flow(piece, width, x)),
                    Part::Group(columns) => {
                        let mut items = Vec::new();
                        for column in columns {
                            let (width, x) = measure(&column);
                            items.extend(column.into_iter().map(|p| to_flow(p, width, x)));
                        }
                        out.push(Flow::Group(items));
                    }
                }
            }
        }
    };
    // The last line of a paragraph across the columns may stop short of
    // the gutter; right under the line before it and starting where it
    // does, it still belongs to that paragraph.
    let mut last_spanning: Option<&PdfLine> = None;
    for piece in pieces {
        let ends_spanning = match (&piece, last_spanning) {
            (Piece::Line(l), Some(prev)) => {
                (l.x - prev.x).abs() <= 1.0 && prev.y - top(l) <= prev.height.max(l.height)
            }
            _ => false,
        };
        let column = if ends_spanning { None } else { column_of(&piece) };
        last_spanning = match (&piece, column) {
            (Piece::Line(l), None) => Some(*l),
            _ => None,
        };
        match column {
            Some(col) => band[col].push(piece),
            None => {
                flush_band(&mut band, &mut out);
                let x = match piece {
                    Piece::Line(line) => line.x,
                    _ => 0.0,
                };
                out.push(to_flow(piece, spanning_width, x));
            }
        }
    }
    flush_band(&mut band, &mut out);
    let out = insert_labels(out, labels, page_index, body);
    defer_footnotes(out, page, body)
}

/// Each label goes right before the first text, figure or table that
/// starts level with it or further down.
fn insert_labels<'a>(
    mut flow: Vec<Flow<'a>>,
    labels: &[Vec<&'a PdfLine>],
    page_index: usize,
    body: f32,
) -> Vec<Flow<'a>> {
    for (n, label) in labels.iter().enumerate() {
        let Some(first) = label.first() else { continue };
        let top = first.y + first.height;
        let at = flow
            .iter()
            .position(|f| f.top() <= top + body * 0.5 && f.text().is_none_or(|p| p.label.is_none()))
            .unwrap_or(flow.len());
        let left = label.iter().map(|l| l.x).fold(f32::MAX, f32::min);
        let width = label.iter().map(|l| l.x + l.width).fold(0.0, f32::max) - left;
        let items = label.iter().map(|line| {
            Flow::Text(Placed {
                line,
                page: page_index,
                column_width: width,
                column_x: left,
                joined: false,
                label: Some((page_index, n)),
            })
        });
        flow.splice(at..at, items);
    }
    flow
}

/// Small text at the foot of the page (footnotes) reads after the body
/// text of the page, not in the middle of the column it sits under. A
/// page that is mostly small text (references) is left alone.
fn defer_footnotes<'a>(flow: Vec<Flow<'a>>, page: &PdfPage, body: f32) -> Vec<Flow<'a>> {
    let body_lines: Vec<&PdfLine> = flow
        .iter()
        .filter_map(Flow::text)
        .map(|p| p.line)
        .filter(|l| l.font_size >= body * 0.85)
        .collect();
    // A footnote has no body text below it in its column (small type in a
    // display equation does).
    let is_note = |f: &Flow| {
        f.text().is_some_and(|p| {
            let l = p.line;
            l.font_size < body * 0.85
                && l.y + l.height < page.height * 0.3
                && !body_lines
                    .iter()
                    .any(|b| b.y + b.height < l.y && b.x < l.x + l.width && b.x + b.width > l.x)
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
/// A figure followed by its caption stays put, and so do slides and
/// tables.
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
        // The figures' caption, if one follows, goes with them.
        let mut resume = end;
        if flow.get(end).and_then(Flow::text).is_some_and(|c| is_caption(&c.line.text)) {
            resume += 1;
            while let (Some(c), Some(next)) = (flow[resume - 1].text(), flow.get(resume).and_then(Flow::text)) {
                if breaks_paragraph(c, next, body) {
                    break;
                }
                resume += 1;
            }
        }
        let before = i.checked_sub(1).and_then(|k| flow[k].text());
        let after = flow.get(resume).and_then(Flow::text);
        let pinned = flow[i..end].iter().any(Flow::pinned);
        // Moving a caption too takes a sentence the figure cut short.
        let cut_short = resume == end || before.is_some_and(|a| !ends_sentence(&a.line.text));
        if let (Some(a), Some(b)) = (before, after)
            && !pinned
            && cut_short
            && !is_caption(&b.line.text)
            && !is_heading(b, body)
            && !breaks_by_shape(a, b, body)
        {
            held_images.extend(i..resume);
            if let Flow::Text(p) = &mut flow[resume] {
                p.joined = true;
            }
        }
        i = resume;
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
            let ends = is_heading(p, body) || prev.is_none_or(|a| breaks_paragraph(a, p, body));
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
    // A gutter has text on both sides: the blank past the end of a column
    // that stops short of the page's right edge (a contents list) is not
    // one.
    gutters.retain(|&(gl, gr)| {
        let left = narrow.iter().filter(|l| l.x + l.width <= gl + 1.0).count();
        let right = narrow.iter().filter(|l| l.x >= gr - 1.0).count();
        left >= 3 && right >= 3
    });
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

/// Text in one inline style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Span {
    pub(crate) text: String,
    pub(crate) bold: bool,
    pub(crate) underline: bool,
}

/// Lines joined into running text: hyphenation at line ends undone, CJK
/// lines joined without a space, the lines' styles kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Para {
    pub(crate) spans: Vec<Span>,
}

impl Para {
    pub(crate) fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn is_blank(&self) -> bool {
        self.spans.iter().all(|s| s.text.trim().is_empty())
    }

    fn push(&mut self, text: &str, bold: bool, underline: bool) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.bold == bold && last.underline == underline => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_string(),
                bold,
                underline,
            }),
        }
    }

    pub(crate) fn push_line(&mut self, line: &PdfLine) {
        let runs = line_runs(line);
        let Some(first) = runs.first().and_then(|r| r.text.chars().next()) else {
            return;
        };
        let last = self.spans.last().and_then(|s| s.text.chars().last());
        match (last, first) {
            (None, _) => {}
            (Some('-'), c) if c.is_lowercase() => {
                if let Some(span) = self.spans.last_mut() {
                    span.text.pop();
                    if span.text.is_empty() {
                        self.spans.pop();
                    }
                }
            }
            (Some(a), b) if is_cjk(a) || is_cjk(b) => {}
            _ => {
                let (bold, underline) = self.spans.last().map_or((false, false), |s| (s.bold, s.underline));
                self.push(" ", bold, underline && runs[0].underline);
            }
        }
        for (i, run) in runs.iter().enumerate() {
            if i > 0 && run.space_before {
                let prev = &runs[i - 1];
                self.push(" ", prev.bold && run.bold, prev.underline && run.underline);
            }
            self.push(&run.text, run.bold, run.underline);
        }
    }

    fn html(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            let mut text = escape_html(&span.text);
            if span.underline {
                text = format!("<u>{text}</u>");
            }
            if span.bold {
                text = format!("<b>{text}</b>");
            }
            out.push_str(&text);
        }
        out
    }
}

/// The runs of a line; a line read without them is one plain run.
pub(crate) fn line_runs(line: &PdfLine) -> std::borrow::Cow<'_, [PdfRun]> {
    if line.runs.is_empty() {
        std::borrow::Cow::Owned(vec![PdfRun {
            text: line.text.trim().to_string(),
            x: line.x,
            width: line.width,
            bold: line.bold,
            underline: false,
            mono: false,
            space_before: false,
        }])
    } else {
        std::borrow::Cow::Borrowed(&line.runs)
    }
}

/// The part of `line` made of `runs` (consecutive runs of it), as a line
/// of its own.
pub(crate) fn sub_line(line: &PdfLine, runs: &[PdfRun]) -> PdfLine {
    let mut text = String::new();
    let mut parts = Vec::with_capacity(runs.len());
    for (i, run) in runs.iter().enumerate() {
        let space_before = i > 0 && run.space_before;
        if space_before {
            text.push(' ');
        }
        text.push_str(&run.text);
        parts.push(PdfRun {
            space_before,
            ..run.clone()
        });
    }
    let left = runs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
    let right = runs.iter().map(|r| r.x + r.width).fold(f32::MIN, f32::max);
    let chars = |bold: bool| -> usize {
        runs.iter()
            .filter(|r| r.bold == bold)
            .map(|r| r.text.chars().count())
            .sum()
    };
    PdfLine {
        text,
        x: left,
        width: right - left,
        bold: chars(true) > chars(false),
        runs: parts,
        ..line.clone()
    }
}

fn top(line: &PdfLine) -> f32 {
    line.y + line.height
}

fn is_body_size(line: &PdfLine, body: f32) -> bool {
    line.font_size >= body * 0.8 && line.font_size < body * HEADING_SCALE
}

fn chars(lines: &[&PdfLine]) -> usize {
    lines.iter().map(|l| l.text.chars().count()).sum()
}

/// Lines wholly left of `edge` (label candidates) and those starting at or
/// right of it (the text they label).
fn label_sides<'a>(lines: &[&'a PdfLine], edge: f32, body: f32) -> (Vec<&'a PdfLine>, Vec<&'a PdfLine>) {
    let left = lines
        .iter()
        .copied()
        .filter(|l| is_body_size(l, body) && l.x + l.width <= edge && l.x < edge - body)
        .collect();
    let right = lines.iter().copied().filter(|l| l.x >= edge).collect();
    (left, right)
}

/// Whether a line stands level with the start of some line beside it.
fn level_with(line: &PdfLine, others: &[&PdfLine], body: f32) -> bool {
    others.iter().any(|o| (top(o) - top(line)).abs() <= body * 0.5)
}

/// Where a column of labels ends, for documents laid out as clauses and
/// definitions: a narrow column at the left holds short headings
/// ("1. 保险合同构成", "38.1 付款担保") level with the first line of the
/// text they head, which fills a much wider column beside it. A page
/// qualifies when its first gutter lies in the left third, the labels are
/// a small part of its text and mostly level with lines beside them; the
/// layout holds for the document when enough pages qualify, and the edge
/// is the middle one of theirs.
fn label_column(pages: &[PdfPage], body: f32) -> Option<f32> {
    let mut edges: Vec<f32> = pages
        .iter()
        .filter_map(|page| {
            let lines: Vec<&PdfLine> = page.lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
            let &(gl, gr) = find_gutters(&lines, &[], page, body).first()?;
            if gl > page.width * 0.35 {
                return None;
            }
            // Just short of where the text beside the labels starts: how
            // far the labels reach varies from page to page.
            let edge = gr - body * 0.75;
            let (left, right) = label_sides(&lines, edge, body);
            if left.len() < 2 || chars(&left) * 3 > chars(&right) {
                return None;
            }
            let level = left.iter().filter(|l| level_with(l, &right, body)).count();
            (level * 10 >= left.len() * 6).then_some(edge)
        })
        .collect();
    let enough = edges.len() >= pages.len().min(2) && edges.len() * 3 >= pages.len();
    if edges.is_empty() || !enough {
        return None;
    }
    edges.sort_by(f32::total_cmp);
    Some(edges[edges.len() / 2])
}

/// The labels on a page, each its lines top-down: a line left of `edge`
/// level with a line beside it starts one, and lines right under it that
/// do not start with a number of their own continue it. A page whose left
/// side is not a few labels (a contents page in two columns) has none.
fn page_labels<'a>(lines: &[&'a PdfLine], edge: f32, body: f32) -> Vec<Vec<&'a PdfLine>> {
    let (mut left, right) = label_sides(lines, edge, body);
    if left.is_empty() || chars(&left) * 3 > chars(&right) {
        return Vec::new();
    }
    left.sort_by(|a, b| top(b).total_cmp(&top(a)));
    let mut labels: Vec<Vec<&PdfLine>> = Vec::new();
    for line in left {
        let continues = labels.last().and_then(|l| l.last()).is_some_and(|prev| {
            let gap = prev.y - top(line);
            gap > -1.0 && gap <= body * 0.6 && enumerator(&line.text).is_none()
        });
        if continues {
            labels.last_mut().expect("a label to continue").push(line);
        } else if level_with(line, &right, body) {
            labels.push(vec![line]);
        }
    }
    labels
}

/// The page with lines that run across the edge of the label column cut
/// there, where a wide gap between two runs of the line lies on the edge:
/// a label set on one baseline with the text beside it. Lines in tables
/// are left to the tables. (Ordinary column gutters are not cut at: the
/// rows of a table drawn without rules cross them the same way.)
fn split_lines(page: &PdfPage, tables: &[Table], body: f32, label_edge: Option<f32>) -> PdfPage {
    let Some(edge) = label_edge else {
        return page.clone();
    };
    let edges = [edge];
    let mut lines = Vec::with_capacity(page.lines.len());
    for line in &page.lines {
        if line.runs.len() < 2 || tables.iter().any(|t| t.holds(line)) {
            lines.push(line.clone());
            continue;
        }
        let mut start = 0;
        for i in 1..line.runs.len() {
            let (a, b) = (&line.runs[i - 1], &line.runs[i]);
            let (gap_left, gap_right) = (a.x + a.width, b.x);
            let cut = gap_right - gap_left >= body * 0.5
                && edges.iter().any(|&e| gap_left <= e && e <= gap_right);
            if cut {
                lines.push(sub_line(line, &line.runs[start..i]));
                start = i;
            }
        }
        if start == 0 {
            lines.push(line.clone());
        } else {
            lines.push(sub_line(line, &line.runs[start..]));
        }
    }
    PdfPage {
        lines,
        ..page.clone()
    }
}

/// A block of the reflowed text, with the page it starts on.
enum Block {
    Heading {
        level: u8,
        text: String,
        page: usize,
    },
    Paragraph {
        para: Para,
        page: usize,
    },
    Image {
        page: usize,
        index: u32,
        width: u32,
        height: u32,
        caption: String,
    },
    Table {
        rows: Vec<Vec<TableCell>>,
        page: usize,
    },
    Code {
        text: String,
        page: usize,
    },
}

/// A cell of a reflowed table.
struct TableCell {
    para: Para,
    colspan: usize,
    rowspan: usize,
    header: bool,
}

impl Block {
    fn page(&self) -> usize {
        match self {
            Block::Heading { page, .. }
            | Block::Paragraph { page, .. }
            | Block::Image { page, .. }
            | Block::Table { page, .. }
            | Block::Code { page, .. } => *page,
        }
    }

    /// Plain text, as conversion gives it back for the block.
    fn text(&self) -> String {
        match self {
            Block::Heading { text, .. } | Block::Code { text, .. } => text.clone(),
            Block::Paragraph { para, .. } => para.text(),
            Block::Image { caption, .. } => caption.clone(),
            Block::Table { rows, .. } => rows
                .iter()
                .map(|r| r.iter().map(|c| c.para.text()).collect::<Vec<_>>().join(" | "))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    fn html(&self) -> String {
        match self {
            Block::Heading { level, text, .. } => {
                format!("<h{level}>{}</h{level}>", escape_html(text))
            }
            Block::Paragraph { para, .. } => format!("<p>{}</p>", para.html()),
            Block::Code { text, .. } => format!("<pre><code>{}</code></pre>", escape_html(text)),
            Block::Table { rows, .. } => {
                let mut out = String::from("<table>");
                for row in rows {
                    out.push_str("<tr>");
                    for cell in row {
                        let tag = if cell.header { "th" } else { "td" };
                        let mut attrs = String::new();
                        if cell.colspan > 1 {
                            attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
                        }
                        if cell.rowspan > 1 {
                            attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
                        }
                        out.push_str(&format!("<{tag}{attrs}>{}</{tag}>", cell.para.html()));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
                out
            }
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
        if let Block::Paragraph { para, .. } = &block
            && is_caption(&para.text())
            && let Some(Block::Image { caption, .. }) = out.last_mut()
            && caption.is_empty()
        {
            *caption = para.text();
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
    // Bold at body size: every word of it, and not a sentence ending in a
    // full stop (bold terms in a line of body text, an emphasised clause).
    let wholly_bold = line_runs(line)
        .iter()
        .filter(|r| r.text.chars().any(char::is_alphabetic))
        .all(|r| r.bold);
    let sentence = line.text.trim_end().ends_with(['。', '！', '？', '；']);
    let bold = line.bold && wholly_bold && !sentence && line.font_size >= body * 0.9;
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
/// weight, one label, or body text.
enum Run<'a> {
    Heading {
        size: u32,
        label: Option<LabelId>,
        lines: Vec<&'a Placed<'a>>,
    },
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
/// and paragraphs, figures become image blocks and tables table blocks.
fn assemble(flow: &[Flow<'_>], body: f32, sizes: &[u32]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text: Vec<&Placed> = Vec::new();
    for item in flow {
        match item {
            Flow::Text(p) => text.push(p),
            Flow::Image { image, page, .. } => {
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
            Flow::Table { table, page } => {
                assemble_text(&text, body, sizes, &mut blocks);
                text.clear();
                blocks.push(table_block(table, *page));
            }
            Flow::Listing { listing, page } => {
                assemble_text(&text, body, sizes, &mut blocks);
                text.clear();
                blocks.push(Block::Code {
                    text: listing.text.clone(),
                    page: *page,
                });
            }
            Flow::Group(items) => {
                assemble_text(&text, body, sizes, &mut blocks);
                text.clear();
                blocks.extend(assemble(items, body, sizes));
            }
        }
    }
    assemble_text(&text, body, sizes, &mut blocks);
    blocks
}

fn table_block(table: &Table, page: usize) -> Block {
    let rows = table
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    let mut para = Para::default();
                    for line in &cell.lines {
                        para.push_line(line);
                    }
                    TableCell {
                        para,
                        colspan: cell.colspan,
                        rowspan: cell.rowspan,
                        header: cell.header,
                    }
                })
                .collect()
        })
        .collect();
    Block::Table { rows, page }
}

/// Whether a placed line reads as a heading: a label, or set as one.
fn is_heading(p: &Placed<'_>, body: f32) -> bool {
    p.label.is_some() || heading_size(p.line, body).is_some()
}

/// Lines set in a typewriter face read as code; the rest as headings and
/// paragraphs.
fn assemble_text(placed: &[&Placed<'_>], body: f32, sizes: &[u32], blocks: &mut Vec<Block>) {
    let code = |p: &Placed| p.label.is_none() && pdf_listing::is_mono(p.line);
    let mut start = 0;
    while start < placed.len() {
        let is_code = code(placed[start]);
        let end = (start..placed.len())
            .find(|&k| code(placed[k]) != is_code)
            .unwrap_or(placed.len());
        let run = &placed[start..end];
        if is_code {
            let lines: Vec<&PdfLine> = run.iter().map(|p| p.line).collect();
            blocks.push(Block::Code {
                text: pdf_listing::code_text(&lines),
                page: run[0].page,
            });
        } else {
            assemble_prose(run, body, sizes, blocks);
        }
        start = end;
    }
}

fn assemble_prose(placed: &[&Placed<'_>], body: f32, sizes: &[u32], blocks: &mut Vec<Block>) {
    let large = |size: u32| size as f32 / 2.0 >= body * HEADING_SCALE;
    let mut candidates: Vec<Option<u32>> = placed
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let mut candidate = heading_size(p.line, body);
            if let Some(size) = candidate
                && !large(size)
            {
                // Bold at body size is a heading only when it stands apart
                // from the paragraphs around it: not a short last line made
                // of a bold term, and not a first line with a run-in bold
                // lead that the next line continues.
                let continues_previous = i > 0
                    && !is_heading(placed[i - 1], body)
                    && !breaks_paragraph(placed[i - 1], p, body);
                let runs_into_next = placed.get(i + 1).is_some_and(|next| {
                    !is_heading(next, body) && !breaks_paragraph(p, next, body)
                });
                if continues_previous || runs_into_next {
                    candidate = None;
                }
            }
            // Bare numbers (page numbers in a contents list) are never
            // headings.
            if !p.line.text.chars().any(char::is_alphabetic) {
                candidate = None;
            }
            candidate
        })
        .collect();
    // A run-in bold lead over several lines: each bold line that reads on
    // into a bold line that is body text is body text too.
    for i in (0..placed.len().saturating_sub(1)).rev() {
        let (p, next) = (placed[i], placed[i + 1]);
        let next_is_lead = candidates[i + 1].is_none()
            && next.label.is_none()
            && heading_size(next.line, body).is_some_and(|s| !large(s));
        if candidates[i].is_some_and(|s| !large(s)) && next_is_lead && !breaks_paragraph(p, next, body) {
            candidates[i] = None;
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    for (&p, candidate) in placed.iter().zip(candidates) {
        if let Some(id) = p.label {
            match runs.last_mut() {
                Some(Run::Heading {
                    label: Some(l), lines, ..
                }) if *l == id => lines.push(p),
                _ => runs.push(Run::Heading {
                    size: 0,
                    label: Some(id),
                    lines: vec![p],
                }),
            }
            continue;
        }
        match candidate {
            Some(size) => {
                if let Some(Run::Heading {
                    size: s,
                    label: None,
                    lines,
                }) = runs.last_mut()
                    && *s == size
                    && lines
                        .last()
                        .is_some_and(|l| l.line.bold == p.line.bold && l.page == p.page)
                {
                    lines.push(p);
                } else {
                    runs.push(Run::Heading {
                        size,
                        label: None,
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
            Run::Heading {
                size,
                label: None,
                lines,
            } if !large(size)
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
    // bold headings at body size rank below every larger one, and labels
    // below those by how deep their numbering goes.
    let level_of = |size: u32| -> u8 {
        let rank = if large(size) {
            sizes.iter().position(|&s| s == size).unwrap_or(sizes.len())
        } else {
            sizes.len()
        };
        (2 + rank.min(2)) as u8
    };
    let label_level = |text: &str| -> u8 {
        let depth = enumerator(text).map_or(1, |e| e.depth.max(1));
        (level_of(0) as usize + depth).min(4) as u8
    };

    for run in &flat {
        match run {
            Run::Heading { size, label, lines } => {
                // Lines of one heading follow each other closely; the same
                // style further down is the next heading.
                let mut start = 0;
                for end in 1..=lines.len() {
                    if end < lines.len() && !breaks_by_gap(lines[end - 1], lines[end]) {
                        continue;
                    }
                    let mut para = Para::default();
                    for p in &lines[start..end] {
                        para.push_line(p.line);
                    }
                    let text = para.text();
                    let level = match label {
                        Some(_) => label_level(&text),
                        None => level_of(*size),
                    };
                    blocks.push(Block::Heading {
                        level,
                        text,
                        page: lines[start].page,
                    });
                    start = end;
                }
            }
            Run::Body(lines) => paragraphs(lines, body, blocks),
        }
    }
}

fn paragraphs(lines: &[&Placed<'_>], body: f32, blocks: &mut Vec<Block>) {
    let mut current = Para::default();
    let mut page = 0;
    let mut prev: Option<&Placed> = None;
    for p in lines {
        if let Some(a) = prev
            && breaks_paragraph(a, p, body)
        {
            flush(&mut current, page, blocks);
        }
        if current.is_blank() {
            page = p.page;
        }
        current.push_line(p.line);
        prev = Some(p);
    }
    flush(&mut current, page, blocks);
}

/// Same page and column, further down: a wide gap is a paragraph space.
fn breaks_by_gap(a: &Placed<'_>, b: &Placed<'_>) -> bool {
    let (la, lb) = (a.line, b.line);
    let line_height = la.height.max(lb.height).max(la.font_size);
    !b.joined && lb.y < la.y && la.y - lb.y > line_height * 1.5
}

/// Whether `b` starts a new paragraph after `a`.
fn breaks_paragraph(a: &Placed<'_>, b: &Placed<'_>, body: f32) -> bool {
    let (la, lb) = (a.line, b.line);
    let line_height = la.height.max(lb.height).max(body);
    if !b.joined && lb.y < la.y && la.y - lb.y > line_height * 1.5 {
        return true;
    }
    breaks_by_shape(a, b, body)
}

/// Paragraph breaks that the lines themselves show, wherever they sit:
/// a change of size or measure, an indent, a new numbered item, a short
/// last line.
fn breaks_by_shape(a: &Placed<'_>, b: &Placed<'_>, body: f32) -> bool {
    let (la, lb) = (a.line, b.line);
    // Right after it on the same row (the parts of a display equation):
    // lines of running text never overlap. A label in the margin stands
    // further off.
    let overlap = (la.y + la.height).min(lb.y + lb.height) - la.y.max(lb.y);
    let gap = lb.x - (la.x + la.width);
    if a.page == b.page && gap >= -1.0 && gap < body * 2.0 && overlap > la.height.min(lb.height) * 0.3 {
        return false;
    }
    if (la.font_size - lb.font_size).abs() > 0.5 {
        return true;
    }
    // A contents entry ends at its page number, set off far to the right.
    let runs = line_runs(la);
    if let [.., before, last] = runs.as_ref()
        && is_number(last.text.trim())
        && last.x - (before.x + before.width) > la.font_size * 2.0
    {
        return true;
    }
    // Moving between a column and full-width text starts a new block.
    if (a.column_width - b.column_width).abs() > body {
        return true;
    }
    let step = lb.x - la.x;
    let numbered_b = enumerator(&lb.text).is_some();
    // Wrapped text never starts with a bullet.
    if lb.text.trim_start().starts_with(BULLETS) {
        return true;
    }
    // A numbered item starts after a line that ends a sentence, where the
    // item before it did or further left; a sub-item stepped in starts
    // after any line. (A wrapped sentence can start "21. The" too.)
    if let Some(e) = enumerator(&lb.text)
        && (ends_sentence(&la.text) || (e.strong && step > body * 0.8))
        && (step <= body * 0.5 || step > body * 0.8)
    {
        return true;
    }
    // A first-line indent: `b` steps in from where the column's lines
    // start. A hanging indent (references): `b` steps back out to the
    // left of where most lines start. A numbered item's own lines step in
    // past its number, and that is not a new paragraph.
    let indent = lb.x - b.column_x;
    let hanging = enumerator(&la.text).is_some() && !numbered_b;
    if step > body * 0.8 && step < a.column_width * 0.5 && indent > body * 0.4 && !hanging {
        return true;
    }
    if step < -body * 0.8 && indent < -body * 0.4 {
        return true;
    }
    // A short line ends its paragraph.
    la.width < a.column_width * SHORT_LINE
}

const BULLETS: [char; 8] = ['•', '●', '○', '◦', '▪', '■', '◆', '‣'];

fn ends_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['。', '；', '：', '！', '？', '.', ';', ':', '!', '?'])
}

/// The number that starts a numbered item or clause.
#[derive(Debug, PartialEq)]
struct Enumerator {
    /// Levels of a dotted number: 1 for "3." or "3)", 3 for "7.2.1.".
    depth: usize,
    /// Set off as a number ("3.", "3)", "（3）"), not a bare "3.2" that a
    /// wrapped sentence could start with ("in Section\n3.2 we").
    strong: bool,
}

/// "1.", "7.2.1.", "38.1 ", "1)", "(1)", "（一）", "a)", "一、", "3、", a bullet, at the
/// start of `text`.
fn enumerator(text: &str) -> Option<Enumerator> {
    let t = text.trim_start();
    let mut chars = t.chars().peekable();
    let first = *chars.peek()?;
    let followed_by_text = |c: Option<char>| c.is_some_and(|c| c.is_whitespace() || is_cjk(c));
    if BULLETS.contains(&first) {
        return Some(Enumerator { depth: 1, strong: true });
    }
    if first == '(' || first == '（' {
        chars.next();
        let inner: String = chars
            .by_ref()
            .take_while(|&c| c != ')' && c != '）')
            .collect();
        let ok = !inner.is_empty()
            && inner.chars().count() <= 3
            && (inner.chars().all(|c| c.is_ascii_digit())
                || inner.chars().all(|c| "一二三四五六七八九十".contains(c)));
        // `take_while` consumed the closing bracket only if there was one.
        return (ok && t[first.len_utf8() + inner.len()..].starts_with([')', '）'])).then_some(Enumerator { depth: 1, strong: true });
    }
    if "一二三四五六七八九十".contains(first) {
        let n = t.chars().take_while(|c| "一二三四五六七八九十".contains(*c)).count();
        return (n <= 3 && t.chars().nth(n) == Some('、')).then_some(Enumerator { depth: 1, strong: true });
    }
    if first.is_ascii_lowercase() {
        let mut it = t.chars();
        it.next();
        return (it.next() == Some(')')).then_some(Enumerator { depth: 1, strong: true });
    }
    let mut depth = 0;
    let mut trailing_dot = false;
    loop {
        let digits = chars.by_ref().take_while_ref(|c| c.is_ascii_digit());
        if digits == 0 || digits > 3 {
            return None;
        }
        depth += 1;
        match chars.peek().copied() {
            Some('.') => {
                chars.next();
                if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    continue;
                }
                trailing_dot = true;
                break;
            }
            Some(')' | '）' | '、') => return Some(Enumerator { depth, strong: true }),
            _ => break,
        }
    }
    let next = chars.next();
    ((trailing_dot || depth >= 2) && followed_by_text(next)).then_some(Enumerator { depth, strong: trailing_dot })
}

/// `take_while` that leaves the first character that fails in place.
trait TakeWhileRef {
    fn take_while_ref(&mut self, f: impl Fn(&char) -> bool) -> usize;
}

impl<I: Iterator<Item = char>> TakeWhileRef for std::iter::Peekable<I> {
    fn take_while_ref(&mut self, f: impl Fn(&char) -> bool) -> usize {
        let mut n = 0;
        while self.next_if(|c| f(c)).is_some() {
            n += 1;
        }
        n
    }
}

/// A paragraph needs a letter or digit: stray punctuation (a dash, a
/// bracket left over from a layout) is not text, and conversion would
/// drop it anyway.
fn flush(current: &mut Para, page: usize, blocks: &mut Vec<Block>) {
    if current.text().chars().any(char::is_alphanumeric) {
        blocks.push(Block::Paragraph {
            para: std::mem::take(current),
            page,
        });
    }
    *current = Para::default();
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
    use crate::import::PdfBox;
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
            runs: Vec::new(),
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
            frames: Vec::new(),
            rules: Vec::new(),
            boxes: Vec::new(),
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
    fn listings_side_by_side_read_one_after_another_after_the_paragraph() {
        // The left column's paragraph runs on at the top of the right
        // column under a figure: two boxed listings, each captioned.
        let mut lines = column(72.0, 220.0, 700.0, 20, "left");
        let mut boxes = Vec::new();
        for (dx, name, sub) in [(0.0, "def a(x):", "(a) First listing"), (112.0, "def b(x):", "(b) Second listing")] {
            let x = 320.0 + dx;
            boxes.push(PdfBox {
                x,
                y: 640.0,
                width: 106.0,
                height: 60.0,
            });
            for (text, indent, y) in [
                (name, 4.0, 688.0),
                ("y ← 0", 8.0, 679.0),
                ("for i in x:", 8.0, 670.0),
                ("y ← y + i", 12.0, 661.0),
                ("return", 8.0, 652.0),
            ] {
                lines.push(line(text, x + indent, y, 7.0));
            }
            lines.push(line(sub, x, 625.0, 8.0));
        }
        lines.push(wide("Figure 3: Two listings", 330.0, 600.0, 200.0, 10.0));
        lines.extend(column(320.0, 220.0, 580.0, 15, "right"));
        let mut p = page(lines);
        p.boxes = boxes;
        let doc = document(&[p]);
        let t = texts(&doc);
        assert_eq!(t.len(), 6, "{t:?}");
        assert!(t[0].starts_with("left line 0") && t[0].ends_with("right line 14 of the column text"));
        assert!(matches!(doc.blocks[1].kind, BlockKind::Code { .. }), "{:?}", doc.blocks[1].kind);
        assert_eq!(t[1], "def a(x):\n  y ← 0\n  for i in x:\n    y ← y + i\n  return");
        assert_eq!(t[2], "(a) First listing");
        assert!(t[3].starts_with("def b(x):"), "{t:?}");
        assert_eq!(t[4], "(b) Second listing");
        assert_eq!(t[5], "Figure 3: Two listings");
    }

    #[test]
    fn typewriter_lines_read_as_code() {
        let mono = |text: &str, x: f32, y: f32| {
            let mut l = line(text, x, y, 10.0);
            l.runs = vec![PdfRun {
                text: text.into(),
                x,
                width: l.width,
                mono: true,
                ..PdfRun::default()
            }];
            l
        };
        let doc = document(&[page(vec![
            wide("Run the command below to start it all up again.", 72.0, 700.0, 300.0, 10.0),
            mono("make build", 72.0, 686.0),
            mono("make run", 72.0, 674.0),
            wide("Then the server listens on its port as before.", 72.0, 660.0, 300.0, 10.0),
        ])]);
        let t = texts(&doc);
        assert_eq!(t.len(), 3, "{t:?}");
        assert!(matches!(doc.blocks[1].kind, BlockKind::Code { .. }));
        assert_eq!(t[1], "make build\nmake run");
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
    fn two_columns_read_in_order_when_the_right_one_ends_short_of_the_edge() {
        // A contents page: both columns of short entries, the right one
        // ending well before the right margin.
        let mut lines = Vec::new();
        for i in 0..20 {
            let y = 740.0 - 26.0 * i as f32;
            lines.push(line(&format!("{}. Left entry", i + 1), 75.0, y, 10.0));
            lines.push(line(&format!("{}. Right entry", i + 21), 337.0, y, 10.0));
        }
        let doc = document(&[page(lines)]);
        let t = texts(&doc).join("\n");
        let pos = |s: &str| t.find(s).unwrap_or_else(|| panic!("{s} in {t}"));
        assert!(pos("20. Left entry") < pos("21. Right entry"), "{t}");
        assert!(pos("1. Left entry") < pos("2. Left entry"), "{t}");
    }

    /// A line read with runs: `(text, x, bold)` each, 5 points a
    /// character, each run after the first set off by a space.
    fn styled(parts: &[(&str, f32, bool)], y: f32) -> PdfLine {
        let runs: Vec<PdfRun> = parts
            .iter()
            .enumerate()
            .map(|(i, &(text, x, bold))| PdfRun {
                text: text.into(),
                x,
                width: text.chars().count() as f32 * 5.0,
                bold,
                underline: false,
                mono: false,
                space_before: i > 0,
            })
            .collect();
        let last = runs.last().unwrap();
        PdfLine {
            text: parts.iter().map(|p| p.0).collect::<Vec<_>>().join(" "),
            x: runs[0].x,
            y,
            width: last.x + last.width - runs[0].x,
            height: 11.5,
            font_size: 10.0,
            bold: parts.iter().all(|p| p.2),
            runs,
        }
    }

    /// A page of clauses: a label column at x=46 and the clauses at 183,
    /// numbered `n.1.` with their own lines stepped in to 211.
    fn clause_page(first: usize) -> PdfPage {
        let body = "clause text that runs on across the measure of the column";
        let more = "and continues on its next line past the clause number here";
        let mut lines = Vec::new();
        for k in 0..5 {
            let n = first + k;
            let top = 680.0 - 110.0 * k as f32;
            if k == 2 {
                // Set on one baseline with its clause, and wrapped.
                lines.push(styled(&[(&format!("{n}. Wrapped"), 46.0, true), (&format!("{n}.1. {body}"), 183.0, false)], top));
                lines.push(bold(line("label end", 60.0, top - 12.0, 10.0)));
            } else {
                lines.push(bold(line(&format!("{n}. Label"), 46.0, top, 10.0)));
                lines.push(wide(&format!("{n}.1. {body}"), 183.0, top, 340.0, 10.0));
            }
            lines.push(wide(more, 211.0, top - 12.0, 330.0, 10.0));
            lines.push(line("Short end.", 211.0, top - 24.0, 10.0));
            lines.push(wide(&format!("{n}.2. {body}"), 183.0, top - 44.0, 340.0, 10.0));
            lines.push(line(&format!("{n}.2.1. a sub-clause."), 211.0, top - 64.0, 10.0));
        }
        page(lines)
    }

    #[test]
    fn labels_in_a_margin_column_head_the_clauses_beside_them() {
        let pages: Vec<PdfPage> = (0..3).map(|p| clause_page(1 + 5 * p)).collect();
        let doc = document(&pages);
        let blocks: Vec<(String, Option<u8>)> = doc
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
        let at = |s: &str| {
            blocks
                .iter()
                .position(|b| b.0.starts_with(s))
                .unwrap_or_else(|| panic!("{s} in {blocks:?}"))
        };
        assert_eq!(blocks[0], ("1. Label".to_string(), Some(3)));
        assert!(blocks[1].0.starts_with("1.1. clause text") && blocks[1].0.ends_with("Short end."), "{:?}", blocks[1]);
        assert_eq!(blocks[1].1, None);
        assert!(at("1.2.") < at("1.2.1.") && at("1.2.1.") < at("2. Label"), "{blocks:?}");
        assert_eq!(blocks[at("3. Wrapped")], ("3. Wrapped label end".to_string(), Some(3)));
        assert!(blocks[at("3. Wrapped") + 1].0.starts_with("3.1. clause"), "{blocks:?}");
        assert!(at("5.2.1.") < at("6. Label"), "{blocks:?}");
        assert_eq!(blocks.iter().filter(|b| b.1.is_some()).count(), 15, "{blocks:?}");
    }

    #[test]
    fn a_contents_page_is_not_a_label_column() {
        let mut pages: Vec<PdfPage> = (0..3).map(|p| clause_page(1 + 5 * p)).collect();
        // Both columns of entries, as many on the left as on the right.
        let mut contents = Vec::new();
        for i in 0..15 {
            let y = 740.0 - 26.0 * i as f32;
            contents.push(line(&format!("{}. Left", i + 1), 75.0, y, 10.0));
            contents.push(line(&format!("{}. Right", i + 16), 337.0, y, 10.0));
        }
        pages.insert(0, page(contents));
        let doc = document(&pages);
        let t = texts(&doc);
        let pos = |s: &str| t.iter().position(|b| b == s).unwrap_or_else(|| panic!("{s} in {t:?}"));
        assert!(pos("1. Left") < pos("2. Left") && pos("15. Left") < pos("16. Right"), "{t:?}");
        assert!(matches!(doc.blocks[pos("1. Left")].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn bold_and_underlined_runs_keep_their_style() {
        let mut l = styled(
            &[("Plain start and", 72.0, false), ("a bold term", 152.0, true), ("then plain again to the end.", 212.0, false)],
            700.0,
        );
        l.runs[2].underline = true;
        let mut lines = vec![l];
        // A wholly bold line ending in a full stop is emphasis, not a heading.
        lines.push(bold(line("您方在犹豫期后解除保险合同会遭受一定损失。", 72.0, 660.0, 10.0)));
        lines.extend(column(72.0, 468.0, 600.0, 10, "pad"));
        let doc = document(&[page(lines)]);
        let runs = doc.blocks[0].kind.runs().unwrap();
        let styled: Vec<(&str, u16)> = runs.iter().map(|r| (r.text.as_str(), r.style)).collect();
        assert_eq!(
            styled,
            [
                ("Plain start and ", 0),
                ("a bold term", eddy_model::style::BOLD),
                (" ", 0),
                ("then plain again to the end.", eddy_model::style::UNDERLINE),
            ]
        );
        assert!(matches!(doc.blocks[1].kind, BlockKind::Paragraph { .. }), "{:?}", doc.blocks[1]);
    }

    #[test]
    fn numbered_items_break_and_their_own_lines_stay_with_them() {
        let lines = vec![
            wide("下列情况下，本保险合同将不续保，具体情形如下所列各项所述的内容范围", 72.0, 700.0, 468.0, 10.0),
            wide("1) 第一项写满了一整行的宽度所以下一行是它自己的续行而不是新的段", 92.0, 688.0, 448.0, 10.0),
            wide("落，这一行缩进到编号之后。", 110.0, 676.0, 130.0, 10.0),
            wide("2) 第二项也写满了整整一行的宽度，下一项紧接着它从同一位置开始；", 92.0, 664.0, 448.0, 10.0),
            wide("3) 第三项。", 92.0, 652.0, 60.0, 10.0),
        ];
        let doc = document(&[page(lines)]);
        assert_eq!(
            texts(&doc),
            [
                "下列情况下，本保险合同将不续保，具体情形如下所列各项所述的内容范围",
                "1) 第一项写满了一整行的宽度所以下一行是它自己的续行而不是新的段落，这一行缩进到编号之后。",
                "2) 第二项也写满了整整一行的宽度，下一项紧接着它从同一位置开始；",
                "3) 第三项。",
            ]
        );
    }

    #[test]
    fn enumerator_shapes() {
        for (s, depth) in [("1. a", 1), ("7.2.1. 本", 3), ("38.1 付款", 2), ("1) x", 1), ("（3）黑", 1), ("(12) y", 1), ("a) z", 1), ("一、总则", 1), ("3、内容", 1), ("1.保险", 1), ("• item", 1)] {
            assert_eq!(enumerator(s).map(|e| e.depth), Some(depth), "{s}");
        }
        for s in ["(c) FVMD versus", "15 天内", "2018年", "3.5%", "and so", "12", "1.2", "Figure 1."] {
            assert_eq!(enumerator(s), None, "{s}");
        }
    }

    #[test]
    fn ruled_tables_become_table_blocks_in_place() {
        use crate::import::PdfRule;
        let mut lines = vec![wide("Text before the table runs the full width of the page.", 72.0, 700.0, 468.0, 10.0)];
        for (r, row) in [["Stage", "T", "N"], ["I", "1", "0"], ["II", "2", "1"]].iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                lines.push(line(cell, 110.0 + 100.0 * c as f32, 655.0 - 20.0 * r as f32, 10.0));
            }
        }
        lines.push(wide("Text after the table runs the full width of the page too.", 72.0, 560.0, 468.0, 10.0));
        let mut p = page(lines);
        for k in 0..4 {
            p.rules.push(PdfRule { x: 100.0, y: 670.0 - 20.0 * k as f32, width: 300.0, height: 0.5 });
            p.rules.push(PdfRule { x: 100.0 + 100.0 * k as f32, y: 610.0, width: 0.5, height: 60.0 });
        }
        let doc = document(&[p]);
        assert_eq!(doc.blocks.len(), 3, "{:?}", texts(&doc));
        assert!(texts(&doc)[0].starts_with("Text before"));
        let BlockKind::Table { rows } = &doc.blocks[1].kind else {
            panic!("{:?}", doc.blocks[1]);
        };
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|r| r.cells.iter().map(|c| eddy_model::runs_plain_text(&c.runs)).collect())
            .collect();
        assert_eq!(cells, [["Stage", "T", "N"], ["I", "1", "0"], ["II", "2", "1"]]);
        assert!(texts(&doc)[2].starts_with("Text after"));
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
    fn slides_printed_with_notes_show_whole_and_the_notes_read_as_text() {
        let pages: Vec<PdfPage> = (0..4)
            .map(|n| {
                let mut p = page(vec![
                    line(&format!("Slide {n} title"), 80.0, 700.0, 22.0),
                    line("Measured", 100.0, 560.0, 9.0),
                    line("Renderer", 300.0, 560.0, 9.0),
                ]);
                p.lines.extend(column(72.0, 468.0, 400.0, 3, &format!("notes{n}")));
                // The template background, the same on every slide, and
                // the slide's fill and border.
                p.images.push(image(2, 60.0, 460.0, 480.0, 270.0, 7));
                p.frames.push(image(1, 60.0, 460.0, 480.0, 270.0, 0));
                p.frames.push(image(9, 59.0, 459.0, 482.0, 272.0, 0));
                p
            })
            .collect();
        let doc = document(&pages);
        let t = texts(&doc);
        assert!(!t.iter().any(|s| s.contains("Slide") || s.contains("Measured")), "{t:?}");
        assert!(t.iter().any(|s| s.starts_with("notes2 line 0")), "{t:?}");
        let figures = images(&doc);
        assert_eq!(figures.len(), 4, "{figures:?}");
        assert_eq!(figures[1].0, "pdf://key/1/9", "the border, which holds the whole slide");
        let slides: Vec<&str> = doc
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Image { .. }))
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(doc.pages, slides, "each page starts at its slide");
    }

    #[test]
    fn a_frame_on_only_some_pages_is_not_a_slide() {
        let mut pages: Vec<PdfPage> = (0..4)
            .map(|n| page(column(72.0, 468.0, 700.0, 5, &format!("p{n}"))))
            .collect();
        pages[1].frames.push(image(3, 60.0, 300.0, 480.0, 270.0, 0));
        pages[1].lines.push(line("Inside a box", 100.0, 400.0, 10.0));
        let doc = document(&pages);
        assert!(images(&doc).is_empty());
        assert!(texts(&doc).iter().any(|s| s.contains("Inside a box")));
    }

    #[test]
    fn each_page_starts_at_its_first_block_or_the_next_one() {
        let mut last = column(72.0, 468.0, 700.0, 3, "first");
        last.push(line("Short end.", 72.0, 664.0, 10.0));
        let mut third = vec![line("Second Part", 72.0, 720.0, 16.0)];
        third.extend(column(72.0, 468.0, 690.0, 3, "third"));
        let pages = [page(last), page(Vec::new()), page(third)];
        let doc = document(&pages);
        let t = texts(&doc);
        let heading = t.iter().position(|s| s == "Second Part").unwrap();
        assert_eq!(doc.pages.len(), 3);
        assert_eq!(doc.pages[0], doc.blocks[0].id);
        assert_eq!(doc.pages[1], doc.blocks[heading].id, "an empty page goes to the next");
        assert_eq!(doc.pages[2], doc.blocks[heading].id);
    }

    #[test]
    fn page_starts_survive_paragraphs_split_around_formulas_and_stray_punctuation() {
        let first = vec![
            wide("Energy is $$E = mc^2$$ which relates mass and energy here.", 72.0, 700.0, 468.0, 10.0),
            line("-", 72.0, 660.0, 10.0),
            line(")", 72.0, 640.0, 10.0),
            line("Short end.", 72.0, 620.0, 10.0),
        ];
        let second = column(72.0, 468.0, 700.0, 3, "second");
        let doc = document(&[page(first), page(second)]);
        let t = texts(&doc);
        assert!(doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Math { .. })), "{t:?}");
        let second = t.iter().position(|s| s.starts_with("second line 0")).unwrap();
        assert_eq!(doc.pages, [doc.blocks[0].id.clone(), doc.blocks[second].id.clone()]);
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
