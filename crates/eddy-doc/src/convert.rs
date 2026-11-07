use std::collections::HashMap;

use eddy_model::{
    Block, BlockKind, Document, MediaKind, Run, TableCell, TableRow, ids::sha256_hex, style,
};
use scraper::{ElementRef, Html, Node};
use url::Url;

use crate::highlight::highlight;
use crate::math;

#[derive(Clone, Debug, Default)]
pub struct ConvertOptions<'a> {
    /// URL the HTML came from; relative links and images resolve against it.
    pub base_url: Option<&'a str>,
    /// Item title. A leading heading repeating it is dropped.
    pub title: Option<&'a str>,
}

/// Converts untrusted HTML into a document. Only semantic structure survives:
/// scripts, styles, forms and presentational markup are dropped.
pub fn html_to_document(html: &str, opts: &ConvertOptions) -> Document {
    let parsed = Html::parse_document(html);
    let base = opts.base_url.and_then(|b| Url::parse(b).ok());
    let mut c = Converter::new(base);
    let root = parsed.root_element();
    let body = root
        .children()
        .filter_map(ElementRef::wrap)
        .find(|e| e.value().name() == "body")
        .unwrap_or(root);
    c.walk_children(body);
    c.flush();

    let mut blocks = math::split_raw_tex(c.blocks);
    drop_title_heading(&mut blocks, opts.title);
    Document {
        schema_version: crate::SCHEMA_VERSION,
        blocks: assign_ids(blocks),
    }
}

struct ListCtx {
    ordered: bool,
    next_index: u32,
}

struct ItemCtx {
    ordered: bool,
    depth: u8,
    index: u32,
    marker_used: bool,
}

struct Converter {
    base: Option<Url>,
    blocks: Vec<(u8, BlockKind)>,
    runs: Vec<Run>,
    style: u16,
    link: Option<String>,
    quote_depth: u8,
    lists: Vec<ListCtx>,
    item: Option<ItemCtx>,
    in_pre: bool,
}

const DISCARD: &[&str] = &[
    "head", "title", "style", "noscript", "template", "form", "input", "button", "select",
    "textarea", "svg", "canvas", "object", "embed", "link", "meta", "base", "map", "dialog", "nav",
    "menu",
];

const BLOCK_CONTAINERS: &[&str] = &[
    "p", "div", "section", "article", "main", "header", "footer", "aside", "address", "center",
    "details", "dl", "dd", "fieldset", "hgroup", "body", "html",
];

impl Converter {
    fn new(base: Option<Url>) -> Self {
        Converter {
            base,
            blocks: Vec::new(),
            runs: Vec::new(),
            style: 0,
            link: None,
            quote_depth: 0,
            lists: Vec::new(),
            item: None,
            in_pre: false,
        }
    }

    fn walk_children(&mut self, el: ElementRef) {
        for child in el.children() {
            match child.value() {
                Node::Text(text) => self.push_text(text),
                Node::Element(_) => {
                    if let Some(e) = ElementRef::wrap(child) {
                        self.element(e);
                    }
                }
                _ => {}
            }
        }
    }

    fn element(&mut self, el: ElementRef) {
        let name = el.value().name();
        if el.value().attr("hidden").is_some() {
            return;
        }
        if let Some(handled) = self.math_element(el)
            && handled
        {
            return;
        }
        if DISCARD.contains(&name) {
            return;
        }
        match name {
            "script" => {}
            "br" => self.push_raw("\n"),
            "hr" => {
                self.flush();
                self.emit(BlockKind::Rule);
            }
            "img" => self.image(el, Vec::new()),
            "picture" => {
                if let Some(img) = first_descendant(el, "img") {
                    self.image(img, Vec::new());
                }
            }
            "figure" => self.figure(el),
            "pre" => self.pre(el),
            "blockquote" => {
                self.flush();
                self.quote_depth = self.quote_depth.saturating_add(1);
                self.walk_children(el);
                self.flush();
                self.quote_depth -= 1;
            }
            "ul" | "ol" | "dir" => {
                self.flush();
                let start = el
                    .value()
                    .attr("start")
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(1);
                self.lists.push(ListCtx {
                    ordered: name == "ol",
                    next_index: start,
                });
                let saved = self.item.take();
                self.walk_children(el);
                self.flush();
                self.item = saved;
                self.lists.pop();
            }
            "li" => self.list_item(el),
            "table" => self.table(el),
            "video" | "audio" => self.media(
                el,
                if name == "video" {
                    MediaKind::Video
                } else {
                    MediaKind::Audio
                },
            ),
            "iframe" => {
                if let Some(src) = el.value().attr("src").and_then(|s| self.resolve(s)) {
                    self.flush();
                    self.emit(BlockKind::Media {
                        media: MediaKind::Embed,
                        src,
                        poster: None,
                        title: el.value().attr("title").map(str::to_string),
                    });
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                let level = name[1..].parse().unwrap_or(2);
                self.walk_children(el);
                let runs = self.take_runs();
                if !runs.is_empty() {
                    self.emit(BlockKind::Heading { level, runs });
                }
            }
            "dt" | "summary" => {
                self.flush();
                self.with_style(style::BOLD, |c| c.walk_children(el));
                self.flush();
            }
            "a" => {
                let href = el
                    .value()
                    .attr("href")
                    .and_then(|h| self.resolve_link(h))
                    .or_else(|| self.link.clone());
                let saved = std::mem::replace(&mut self.link, href);
                self.walk_children(el);
                self.link = saved;
            }
            "q" => {
                self.push_text("“");
                self.walk_children(el);
                self.push_text("”");
            }
            _ => {
                if let Some(flag) = inline_style(name) {
                    self.with_style(flag, |c| c.walk_children(el));
                } else if BLOCK_CONTAINERS.contains(&name) {
                    self.flush();
                    self.walk_children(el);
                    self.flush();
                } else {
                    self.walk_children(el);
                }
            }
        }
    }

    /// Returns `Some(true)` when the element was consumed as a formula.
    fn math_element(&mut self, el: ElementRef) -> Option<bool> {
        let name = el.value().name();
        let classes: Vec<&str> = el.value().classes().collect();
        if classes.iter().any(|c| {
            *c == "MathJax_Preview"
                || *c == "MathJax"
                || *c == "MathJax_Display"
                || *c == "MathJax_SVG"
        }) {
            return Some(true);
        }
        let (tex, display) = if classes.contains(&"katex-display") {
            (math::tex_in(el)?, true)
        } else if classes.contains(&"katex") {
            (math::tex_in(el)?, false)
        } else if name == "mjx-container" {
            let display = el.value().attr("display") == Some("true");
            (math::tex_in(el)?, display)
        } else if name == "math" {
            let display = el.value().attr("display") == Some("block");
            (math::tex_in(el)?, display)
        } else if name == "script" {
            let ty = el.value().attr("type")?;
            if !ty.starts_with("math/tex") {
                return None;
            }
            let tex = el.text().collect::<String>().trim().to_string();
            (tex, ty.contains("mode=display"))
        } else {
            return None;
        };
        if tex.is_empty() {
            return Some(true);
        }
        if display {
            self.flush();
            self.emit(BlockKind::Math { tex });
        } else {
            let mut run = Run::math(tex);
            run.link = self.link.clone();
            self.runs.push(run);
        }
        Some(true)
    }

    fn with_style(&mut self, flag: u16, f: impl FnOnce(&mut Self)) {
        let saved = self.style;
        self.style |= flag;
        f(self);
        self.style = saved;
    }

    fn push_text(&mut self, text: &str) {
        if self.in_pre {
            self.push_raw(text);
            return;
        }
        let mut normalized = String::with_capacity(text.len());
        let mut last_space = self.ends_with_space();
        for ch in text.chars() {
            if ch.is_whitespace() && ch != '\u{a0}' {
                if !last_space {
                    normalized.push(' ');
                    last_space = true;
                }
            } else {
                normalized.push(ch);
                last_space = false;
            }
        }
        if !normalized.is_empty() {
            self.push_raw(&normalized);
        }
    }

    fn ends_with_space(&self) -> bool {
        match self.runs.last() {
            None => true,
            Some(r) => r.text.ends_with(' ') || r.text.ends_with('\n'),
        }
    }

    fn push_raw(&mut self, text: &str) {
        if let Some(last) = self.runs.last_mut()
            && !last.math
            && last.style == self.style
            && last.link == self.link
        {
            last.text.push_str(text);
            return;
        }
        self.runs.push(Run {
            text: text.to_string(),
            style: self.style,
            link: self.link.clone(),
            math: false,
        });
    }

    fn take_runs(&mut self) -> Vec<Run> {
        let mut runs = std::mem::take(&mut self.runs);
        if let Some(first) = runs.iter_mut().find(|r| !r.math) {
            let trimmed = first.text.trim_start_matches([' ', '\n']).to_string();
            first.text = trimmed;
        }
        if let Some(last) = runs.iter_mut().rev().find(|r| !r.math) {
            let trimmed = last.text.trim_end_matches([' ', '\n']).to_string();
            last.text = trimmed;
        }
        runs.retain(|r| r.math || !r.text.is_empty());
        if runs.iter().all(|r| !r.math && r.text.trim().is_empty()) {
            return Vec::new();
        }
        runs
    }

    fn flush(&mut self) {
        let runs = self.take_runs();
        if runs.is_empty() {
            return;
        }
        let kind = match self.item.as_mut() {
            Some(item) => {
                let marker = !item.marker_used;
                item.marker_used = true;
                BlockKind::ListItem {
                    ordered: item.ordered,
                    depth: item.depth,
                    index: item.index,
                    marker,
                    runs,
                }
            }
            None => BlockKind::Paragraph { runs },
        };
        self.emit(kind);
    }

    fn emit(&mut self, kind: BlockKind) {
        self.blocks.push((self.quote_depth, kind));
    }

    fn list_item(&mut self, el: ElementRef) {
        self.flush();
        let depth = self.lists.len().saturating_sub(1) as u8;
        let (ordered, index) = match self.lists.last_mut() {
            Some(list) => {
                let index = el
                    .value()
                    .attr("value")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(list.next_index);
                list.next_index = index + 1;
                (list.ordered, index)
            }
            None => (false, 1),
        };
        let saved = self.item.replace(ItemCtx {
            ordered,
            depth,
            index,
            marker_used: false,
        });
        self.walk_children(el);
        self.flush();
        self.item = saved;
    }

    fn image(&mut self, el: ElementRef, caption: Vec<Run>) {
        let Some(src) = image_source(el).and_then(|s| self.resolve(&s)) else {
            return;
        };
        let width = dimension(el, "width");
        let height = dimension(el, "height");
        if width.is_some_and(|w| w <= 2) || height.is_some_and(|h| h <= 2) {
            return;
        }
        self.flush();
        let alt = el
            .value()
            .attr("alt")
            .unwrap_or("")
            .trim()
            .chars()
            .take(500)
            .collect();
        self.emit(BlockKind::Image {
            src,
            alt,
            width,
            height,
            caption,
        });
    }

    fn figure(&mut self, el: ElementRef) {
        self.flush();
        let caption = el
            .children()
            .filter_map(ElementRef::wrap)
            .find(|e| e.value().name() == "figcaption")
            .map(|cap| self.collect_runs(cap))
            .unwrap_or_default();
        let before = self.blocks.len();
        for child in el.children() {
            match child.value() {
                Node::Element(e) if e.name() == "figcaption" => {}
                Node::Element(_) => {
                    if let Some(e) = ElementRef::wrap(child) {
                        self.element(e);
                    }
                }
                Node::Text(t) => self.push_text(t),
                _ => {}
            }
        }
        self.flush();
        if caption.is_empty() {
            return;
        }
        let attached = self.blocks[before..]
            .iter_mut()
            .rev()
            .find_map(|(_, kind)| match kind {
                BlockKind::Image { caption: c, .. } if c.is_empty() => Some(c),
                _ => None,
            });
        match attached {
            Some(slot) => *slot = caption,
            None => {
                let runs = caption.into_iter().map(|mut r| {
                    r.style |= style::SMALL;
                    r
                });
                self.emit(BlockKind::Paragraph {
                    runs: runs.collect(),
                });
            }
        }
    }

    fn pre(&mut self, el: ElementRef) {
        self.flush();
        let language = code_language(el);
        let mut text = String::new();
        collect_pre_text(el, &mut text);
        let text = text
            .trim_end_matches('\n')
            .trim_start_matches('\n')
            .to_string();
        if text.trim().is_empty() {
            return;
        }
        let highlights = language
            .as_deref()
            .map(|l| highlight(&text, l))
            .unwrap_or_default();
        self.emit(BlockKind::Code {
            language,
            text,
            highlights,
        });
    }

    fn table(&mut self, el: ElementRef) {
        self.flush();
        let rows: Vec<ElementRef> = table_rows(el);
        let cell_count: usize = rows.iter().map(|r| cells(*r).len()).sum();
        let nested = first_descendant(el, "table").is_some();
        if nested || cell_count <= 1 {
            // Layout table: keep its content as ordinary blocks.
            self.walk_children(el);
            self.flush();
            return;
        }
        let mut out = Vec::new();
        for row in rows {
            let cells = cells(row)
                .into_iter()
                .map(|cell| TableCell {
                    header: cell.value().name() == "th",
                    colspan: span(cell, "colspan"),
                    rowspan: span(cell, "rowspan"),
                    runs: self.collect_runs(cell),
                })
                .collect::<Vec<_>>();
            if !cells.is_empty() {
                out.push(TableRow { cells });
            }
        }
        if !out.is_empty() {
            self.emit(BlockKind::Table { rows: out });
        }
    }

    fn media(&mut self, el: ElementRef, media: MediaKind) {
        let src = el
            .value()
            .attr("src")
            .map(str::to_string)
            .or_else(|| {
                first_descendant(el, "source")
                    .and_then(|s| s.value().attr("src").map(str::to_string))
            })
            .and_then(|s| self.resolve(&s));
        let Some(src) = src else { return };
        self.flush();
        let poster = el.value().attr("poster").and_then(|p| self.resolve(p));
        let title = el.value().attr("title").map(str::to_string);
        self.emit(BlockKind::Media {
            media,
            src,
            poster,
            title,
        });
    }

    /// Converts an element's content into inline runs without emitting blocks.
    fn collect_runs(&mut self, el: ElementRef) -> Vec<Run> {
        let saved_runs = std::mem::take(&mut self.runs);
        let saved_blocks = self.blocks.len();
        let saved_item = self.item.take();
        self.walk_children(el);
        // Block-level content inside a cell or caption collapses to text.
        let extra: Vec<(u8, BlockKind)> = self.blocks.drain(saved_blocks..).collect();
        let mut runs = Vec::new();
        for (_, kind) in extra {
            if let Some(r) = kind.runs() {
                if !runs.is_empty() {
                    runs.push(Run::text(" "));
                }
                runs.extend(r.iter().cloned());
            }
        }
        if !runs.is_empty() && !self.runs.is_empty() {
            runs.push(Run::text(" "));
        }
        runs.extend(std::mem::take(&mut self.runs));
        self.runs = runs;
        let out = self.take_runs();
        self.runs = saved_runs;
        self.item = saved_item;
        out
    }

    fn resolve(&self, href: &str) -> Option<String> {
        let href = href.trim();
        if href.is_empty() || href.starts_with("data:") {
            return None;
        }
        let url = match &self.base {
            Some(base) => base.join(href).ok()?,
            None => Url::parse(href).ok()?,
        };
        matches!(url.scheme(), "http" | "https").then(|| url.to_string())
    }

    fn resolve_link(&self, href: &str) -> Option<String> {
        let href = href.trim();
        if href.starts_with("mailto:") {
            return Some(href.to_string());
        }
        if href.starts_with('#') {
            return None;
        }
        self.resolve(href)
    }
}

fn inline_style(name: &str) -> Option<u16> {
    Some(match name {
        "strong" | "b" => style::BOLD,
        "em" | "i" | "cite" | "dfn" | "var" => style::ITALIC,
        "code" | "kbd" | "samp" | "tt" => style::CODE,
        "s" | "del" | "strike" => style::STRIKE,
        "u" | "ins" => style::UNDERLINE,
        "sup" => style::SUPERSCRIPT,
        "sub" => style::SUBSCRIPT,
        "small" => style::SMALL,
        "mark" => style::MARK,
        _ => return None,
    })
}

fn first_descendant<'a>(el: ElementRef<'a>, name: &str) -> Option<ElementRef<'a>> {
    el.descendants()
        .skip(1)
        .filter_map(ElementRef::wrap)
        .find(|e| e.value().name() == name)
}

fn image_source(el: ElementRef) -> Option<String> {
    let attr = |name: &str| {
        el.value()
            .attr(name)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let src = attr("src");
    let placeholder = src.is_none_or(|s| {
        s.starts_with("data:") || s.contains("placeholder") || s.contains("blank.gif")
    });
    if placeholder {
        for name in ["data-src", "data-original", "data-lazy-src", "data-url"] {
            if let Some(v) = attr(name) {
                return Some(v.to_string());
            }
        }
        for name in ["data-srcset", "srcset"] {
            if let Some(v) = attr(name).and_then(largest_srcset) {
                return Some(v);
            }
        }
    }
    src.map(str::to_string)
}

fn largest_srcset(srcset: &str) -> Option<String> {
    srcset
        .split(',')
        .filter_map(|candidate| {
            let mut parts = candidate.split_whitespace();
            let url = parts.next()?;
            let weight = parts
                .next()
                .and_then(|d| d.trim_end_matches(['w', 'x']).parse::<f64>().ok())
                .unwrap_or(1.0);
            Some((weight, url.to_string()))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, url)| url)
}

fn dimension(el: ElementRef, name: &str) -> Option<u32> {
    el.value()
        .attr(name)?
        .trim()
        .trim_end_matches("px")
        .parse()
        .ok()
}

fn span(el: ElementRef, name: &str) -> u16 {
    el.value()
        .attr(name)
        .and_then(|v| v.trim().parse().ok())
        .filter(|v| (1..=50).contains(v))
        .unwrap_or(1)
}

fn code_language(pre: ElementRef) -> Option<String> {
    let mut candidates = vec![pre];
    if let Some(code) = first_descendant(pre, "code") {
        candidates.push(code);
    }
    for el in candidates {
        if let Some(lang) = el
            .value()
            .attr("data-lang")
            .or(el.value().attr("data-language"))
        {
            return Some(lang.to_lowercase());
        }
        for class in el.value().classes() {
            for prefix in ["language-", "lang-", "highlight-source-", "brush:"] {
                if let Some(rest) = class.strip_prefix(prefix)
                    && !rest.is_empty()
                {
                    return Some(rest.to_lowercase());
                }
            }
        }
    }
    None
}

fn collect_pre_text(el: ElementRef, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if e.name() == "br" => out.push('\n'),
            Node::Element(_) => {
                if let Some(e) = ElementRef::wrap(child) {
                    collect_pre_text(e, out);
                    if matches!(e.value().name(), "div" | "p") && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
            _ => {}
        }
    }
}

fn table_rows(table: ElementRef) -> Vec<ElementRef> {
    let mut rows = Vec::new();
    for child in table.children().filter_map(ElementRef::wrap) {
        match child.value().name() {
            "tr" => rows.push(child),
            "thead" | "tbody" | "tfoot" => rows.extend(
                child
                    .children()
                    .filter_map(ElementRef::wrap)
                    .filter(|e| e.value().name() == "tr"),
            ),
            _ => {}
        }
    }
    rows
}

fn cells(row: ElementRef) -> Vec<ElementRef> {
    row.children()
        .filter_map(ElementRef::wrap)
        .filter(|e| matches!(e.value().name(), "td" | "th"))
        .collect()
}

fn drop_title_heading(blocks: &mut Vec<(u8, BlockKind)>, title: Option<&str>) {
    let Some(title) = title.map(normalize_title).filter(|t| !t.is_empty()) else {
        return;
    };
    if let Some(pos) = blocks
        .iter()
        .position(|(_, k)| !matches!(k, BlockKind::Rule))
        && let (_, BlockKind::Heading { runs, .. }) = &blocks[pos]
        && normalize_title(&eddy_model::runs_plain_text(runs)) == title
        && pos < 3
    {
        blocks.remove(pos);
    }
}

fn normalize_title(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Gives every block a stable ID derived from its kind and content, so editing
/// one block leaves the others' IDs (and anchors pointing at them) intact.
fn assign_ids(blocks: Vec<(u8, BlockKind)>) -> Vec<Block> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    blocks
        .into_iter()
        .map(|(quote_depth, kind)| {
            let key = match &kind {
                BlockKind::Image { src, .. } | BlockKind::Media { src, .. } => src.clone(),
                _ => Block {
                    id: String::new(),
                    quote_depth,
                    kind: kind.clone(),
                }
                .plain_text(),
            };
            let digest = sha256_hex(
                format!(
                    "{}\u{1f}{}",
                    kind.tag(),
                    key.split_whitespace().collect::<Vec<_>>().join(" ")
                )
                .as_bytes(),
            );
            let base = format!("b{}", &digest[..10]);
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            let id = if *n == 1 { base } else { format!("{base}-{n}") };
            Block {
                id,
                quote_depth,
                kind,
            }
        })
        .collect()
}
