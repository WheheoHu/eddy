use std::collections::HashSet;

use scraper::{ElementRef, Html, Selector};
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub struct ChapterLink {
    pub title: String,
    pub url: String,
    /// Depth in the table of contents: 0 for a chapter, 1 for a section of
    /// one, and so on. From nested lists, else from dotted numbering.
    pub level: u32,
}

/// A proposed series built from a table-of-contents page, for the user to
/// confirm.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesProposal {
    pub title: String,
    pub toc_url: String,
    pub chapters: Vec<ChapterLink>,
}

const MIN_CHAPTERS: usize = 4;
const MAX_CHAPTERS: usize = 500;

/// Looks for a table of contents on the page: a link list whose links stay
/// within the page's section of the site (docs books, tutorial series).
pub fn detect_series(html: &str, url: &str) -> Option<SeriesProposal> {
    let base = Url::parse(url).ok()?;
    let doc = Html::parse_document(html);
    // The page's own directory first, then the site section it belongs to.
    let chapters = section_prefixes(&base)
        .into_iter()
        .find_map(|prefix| chapters_under(&doc, &base, &prefix))?;
    let title =
        crate::page_title(&doc).unwrap_or_else(|| base.host_str().unwrap_or("Series").to_string());
    Some(SeriesProposal {
        title,
        toc_url: url.to_string(),
        chapters,
    })
}

fn chapters_under(doc: &Html, base: &Url, prefix: &str) -> Option<Vec<ChapterLink>> {
    let candidates = Selector::parse("nav, ol, ul, aside, div, section").unwrap();
    let links = Selector::parse("a[href]").unwrap();

    let mut best: Option<(f64, Vec<ChapterLink>)> = None;
    for el in doc.select(&candidates) {
        let all: Vec<ElementRef> = el.select(&links).collect();
        if all.len() < MIN_CHAPTERS {
            continue;
        }
        let mut seen = HashSet::new();
        let mut chapters = Vec::new();
        for a in &all {
            let Some(href) = a.value().attr("href") else {
                continue;
            };
            let Ok(mut target) = base.join(href) else {
                continue;
            };
            target.set_fragment(None);
            if target.host_str() != base.host_str() || !target.path().starts_with(prefix) {
                continue;
            }
            let title = a
                .text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if title.is_empty() || !seen.insert(target.to_string()) {
                continue;
            }
            chapters.push(ChapterLink {
                title,
                url: target.to_string(),
                level: list_depth(*a, el),
            });
            if chapters.len() >= MAX_CHAPTERS {
                break;
            }
        }
        // Compare against distinct pages: tables of contents link sections
        // of the same chapter with #anchors.
        let distinct: HashSet<String> = all
            .iter()
            .filter_map(|a| base.join(a.value().attr("href")?).ok())
            .map(|mut u| {
                u.set_fragment(None);
                u.to_string()
            })
            .collect();
        let ratio = chapters.len() as f64 / distinct.len().max(1) as f64;
        if chapters.len() < MIN_CHAPTERS || ratio < 0.6 {
            continue;
        }
        let mut score = chapters.len() as f64 * ratio;
        if looks_like_toc(el) {
            score += 5.0;
        }
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, chapters));
        }
    }
    best.map(|(_, mut chapters)| {
        level_chapters(&mut chapters);
        chapters
    })
}

/// How many lists lie between a link and the container it was found in.
fn list_depth(link: ElementRef, container: ElementRef) -> u32 {
    let mut depth = 0;
    let mut node = link.parent().and_then(ElementRef::wrap);
    while let Some(e) = node {
        if e.id() == container.id() {
            break;
        }
        if matches!(e.value().name(), "ul" | "ol") {
            depth += 1;
        }
        node = e.parent().and_then(ElementRef::wrap);
    }
    depth
}

/// Levels count from the shallowest link. A flat list numbered "1", "1.1",
/// "1.1.1" takes its levels from the numbering instead.
fn level_chapters(chapters: &mut [ChapterLink]) {
    let min = chapters.iter().map(|c| c.level).min().unwrap_or(0);
    for c in chapters.iter_mut() {
        c.level -= min;
    }
    if chapters.iter().any(|c| c.level > 0) {
        return;
    }
    let numbered: Vec<Option<u32>> = chapters.iter().map(|c| numbering_depth(&c.title)).collect();
    // Most chapters carry a number, and some of them have more than one part.
    let with = numbered.iter().filter(|n| n.is_some()).count();
    let nested = numbered.iter().any(|n| n.is_some_and(|d| d > 1));
    if with * 3 < chapters.len() * 2 || !nested {
        return;
    }
    let top = numbered.iter().flatten().min().copied().unwrap_or(1);
    let mut last = 0;
    for (c, n) in chapters.iter_mut().zip(numbered) {
        // An unnumbered title between numbered ones stays with its chapter.
        last = n.map(|d| d - top).unwrap_or(last);
        c.level = last;
    }
}

/// How many parts the number starting a title has: "3" is 1, "3.2" is 2,
/// "3.2.1" is 3; `None` without a number.
fn numbering_depth(title: &str) -> Option<u32> {
    let mut rest = title.trim_start();
    for prefix in ["Chapter ", "Section ", "Part ", "Ch. ", "§"] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.trim_start();
        }
    }
    let number: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let number = number.trim_end_matches('.');
    if number.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    // The number has to end before a word: "2024 in review" isn't a chapter 2024.
    let after = rest[number.len()..].trim_start_matches('.');
    if !after.is_empty()
        && !after.starts_with(|c: char| c.is_whitespace() || c == ':' || c == ')' || c == '-')
    {
        return None;
    }
    if number.split('.').any(|p| p.is_empty()) {
        return None;
    }
    Some(number.split('.').count() as u32)
}

/// Candidate path prefixes chapter links live under: the page's directory,
/// then its first path segment.
fn section_prefixes(url: &Url) -> Vec<String> {
    let path = url.path();
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    let first_dir = if first.is_empty() || !path.trim_start_matches('/').contains('/') {
        "/".to_string()
    } else {
        format!("/{first}/")
    };
    let mut out = vec![dir.to_string()];
    if first_dir != dir {
        out.push(first_dir);
    }
    out
}

fn looks_like_toc(el: ElementRef) -> bool {
    let mut node = Some(el);
    let mut depth = 0;
    while let Some(e) = node {
        let attrs = format!(
            "{} {}",
            e.value().attr("class").unwrap_or(""),
            e.value().attr("id").unwrap_or("")
        )
        .to_lowercase();
        if [
            "toc", "contents", "chapter", "summary", "sidebar", "table-of", "book",
        ]
        .iter()
        .any(|k| attrs.contains(k))
        {
            return true;
        }
        depth += 1;
        if depth > 4 {
            break;
        }
        node = e.parent().and_then(ElementRef::wrap);
    }
    false
}
