//! Full-text extraction: site rules first, Mozilla Readability (via
//! dom_smoothie) otherwise. Also hosts truncation detection and series
//! (table of contents) detection.

mod series;
mod truncation;

use std::sync::LazyLock;

use eddy_fetch::SiteRule;
use regex::Regex;
use scraper::{Html, Selector};
use url::Url;

pub use series::{ChapterLink, SeriesProposal, detect_series};
pub use truncation::{is_truncated, truncation_hint};

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("没有在页面里找到正文")]
    NoContent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Extracted {
    pub title: Option<String>,
    pub byline: Option<String>,
    /// Article HTML, ready for `eddy_doc::html_to_document`.
    pub content_html: String,
    /// Readable characters (no whitespace) in the article.
    pub text_len: usize,
    pub excerpt: Option<String>,
    pub published: Option<String>,
    pub site_name: Option<String>,
    pub next_page: Option<String>,
}

/// Extracts the article from a full page fetched from `url`.
pub fn extract(html: &str, url: &str, rule: Option<&SiteRule>) -> Result<Extracted, ExtractError> {
    let html = preserve_math(html);
    let mut doc = Html::parse_document(&html);
    let heading = select_text(&doc, "h1");
    let title = rule
        .and_then(|r| r.title.as_deref())
        .and_then(|sel| select_text(&doc, sel))
        .or_else(|| page_title(&doc))
        .map(|t| clean_title(t, heading.as_deref()));
    let next_page = rule.and_then(|r| r.next_page.as_deref()).and_then(|sel| select_href(&doc, sel, url));

    if let Some(rule) = rule {
        strip(&mut doc, &rule.strip);
        if !rule.body.is_empty() {
            let parts: Vec<String> = rule
                .body
                .iter()
                .filter_map(|s| Selector::parse(s).ok())
                .flat_map(|sel| doc.select(&sel).map(|e| e.html()).collect::<Vec<_>>())
                .collect();
            if !parts.is_empty() {
                let content_html = parts.join("\n");
                let text_len = text_len_of(&content_html);
                if text_len > 0 {
                    return Ok(Extracted {
                        title,
                        byline: None,
                        content_html,
                        text_len,
                        excerpt: None,
                        published: None,
                        site_name: None,
                        next_page,
                    });
                }
            }
        }
    }

    let source = doc.html();
    let cfg = dom_smoothie::Config { max_elements_to_parse: 0, ..Default::default() };
    let readability = dom_smoothie::Readability::new(source.as_str(), Url::parse(url).ok().map(|_| url), Some(cfg));
    match readability.and_then(|mut r| r.parse()) {
        Ok(article) => {
            let content_html = article.content.to_string();
            let text_len = article.text_content.chars().filter(|c| !c.is_whitespace()).count();
            if text_len == 0 {
                return Err(ExtractError::NoContent);
            }
            Ok(Extracted {
                title: Some(article.title)
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| clean_title(t, heading.as_deref()))
                    .or(title),
                byline: article.byline,
                content_html,
                text_len,
                excerpt: article.excerpt,
                published: article.published_time,
                site_name: article.site_name,
                next_page,
            })
        }
        Err(_) => {
            // Readability gave up (short page): keep the whole body.
            let body = Selector::parse("body").unwrap();
            let content_html = doc.select(&body).next().map(|b| b.inner_html()).ok_or(ExtractError::NoContent)?;
            let text_len = text_len_of(&content_html);
            if text_len == 0 {
                return Err(ExtractError::NoContent);
            }
            Ok(Extracted { title, byline: None, content_html, text_len, excerpt: None, published: None, site_name: None, next_page })
        }
    }
}

/// The page `<title>` (or `og:title`), trimmed of site suffixes like " | Site".
pub fn page_title(doc: &Html) -> Option<String> {
    let og = Selector::parse("meta[property=\"og:title\"]").unwrap();
    if let Some(t) = doc.select(&og).next().and_then(|m| m.value().attr("content")) {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    let title = Selector::parse("title").unwrap();
    let t = doc.select(&title).next()?.text().collect::<String>();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

/// Drops site names around a title ("Post | Site") when the page's main
/// heading shows which part is the real title.
fn clean_title(title: String, heading: Option<&str>) -> String {
    match heading.map(str::trim) {
        Some(h) if !h.is_empty() && h.len() < title.len() && title.contains(h) => h.to_string(),
        _ => title,
    }
}

pub fn page_title_of(html: &str) -> Option<String> {
    page_title(&Html::parse_document(html))
}

fn select_text(doc: &Html, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    let t: String = doc.select(&sel).next()?.text().collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

fn select_href(doc: &Html, selector: &str, base: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    let href = doc.select(&sel).next()?.value().attr("href")?;
    Url::parse(base).ok()?.join(href).ok().map(|u| u.to_string())
}

fn strip(doc: &mut Html, selectors: &[String]) {
    let mut ids = Vec::new();
    for s in selectors {
        if let Ok(sel) = Selector::parse(s) {
            ids.extend(doc.select(&sel).map(|e| e.id()));
        }
    }
    for id in ids {
        if let Some(mut node) = doc.tree.get_mut(id) {
            node.detach();
        }
    }
}

fn text_len_of(html: &str) -> usize {
    Html::parse_fragment(html).root_element().text().flat_map(str::chars).filter(|c| !c.is_whitespace()).count()
}

static MATH_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<script[^>]*type\s*=\s*["']math/tex(; *mode=display)?["'][^>]*>(.*?)</script>"#).unwrap());
static MATHML: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<math\b([^>]*)>(.*?)</math>"#).unwrap());
static TEX_ANNOTATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<annotation[^>]*encoding\s*=\s*["'](?:application/x-tex|tex)["'][^>]*>(.*?)</annotation>"#).unwrap());
static ALTTEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\balttext\s*=\s*"([^"]*)""#).unwrap());
static MATH_FALLBACK_IMG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<img\b[^>]*class\s*=\s*["'][^"']*mwe-math-fallback[^"']*["'][^>]*>"#).unwrap());
/// Wikipedia hides the accessible MathML (`mwe-math-mathml-*`) behind
/// `display: none`, and Readability drops hidden elements.
static HIDDEN_MATH_WRAPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)(<span\b[^>]*class\s*=\s*["'][^"']*mwe-math-mathml[^"']*["'][^>]*?)\s*style\s*=\s*["']\s*display:\s*none;?\s*["']"#).unwrap());

/// Readability drops `<script>` elements and hidden MathML, which would take
/// MathJax and MathML formulas with them. Rewrites both as TeX delimiters in
/// text, which the document converter turns back into math, and drops the
/// image fallbacks that duplicate them.
pub fn preserve_math(html: &str) -> String {
    let mut out = html.to_string();
    if out.contains("math/tex") {
        out = MATH_SCRIPT
            .replace_all(&out, |c: &regex::Captures| {
                let tex = c[2].trim().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                delimit(&tex, c.get(1).is_some())
            })
            .into_owned();
    }
    if out.contains("<math") {
        out = MATHML
            .replace_all(&out, |c: &regex::Captures| {
                // Annotation text is already escaped for an HTML text context.
                let tex = TEX_ANNOTATION
                    .captures(&c[2])
                    .map(|a| a[1].trim().to_string())
                    .or_else(|| ALTTEXT.captures(&c[1]).map(|a| a[1].trim().to_string()))
                    .or_else(|| {
                        DATA_MW.captures(&c[1]).and_then(|a| {
                            let raw = a.get(1).or(a.get(2))?.as_str();
                            let tex = eddy_doc::tex_from_data_mw(&decode_entities(raw))?;
                            Some(tex.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
                        })
                    });
                match tex.filter(|t| !t.is_empty()) {
                    Some(tex) => delimit(&tex, c[1].contains("display=\"block\"")),
                    None => c[0].to_string(),
                }
            })
            .into_owned();
        out = MATH_FALLBACK_IMG.replace_all(&out, "").into_owned();
        out = HIDDEN_MATH_WRAPPER.replace_all(&out, "$1").into_owned();
    }
    out
}

static DATA_MW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\bdata-mw\s*=\s*(?:'([^']*)'|"([^"]*)")"#).unwrap());

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn delimit(tex: &str, display: bool) -> String {
    if display { format!("<div>\\[{tex}\\]</div>") } else { format!("\\({tex}\\)") }
}

#[cfg(test)]
mod tests;
