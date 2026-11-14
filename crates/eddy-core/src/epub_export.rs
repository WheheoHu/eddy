use std::collections::HashMap;
use std::io::{Cursor, Write};

use eddy_doc::translated_runs;
use eddy_model::{
    Block, BlockKind, Document, HighlightSpan, MediaKind, Run, TableRow, TokenClass,
    runs_plain_text, style,
};
use uuid::Uuid;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::export::{ExportContentMode, PreparedExportChapter};
use crate::{EngineError, Result};

pub(crate) struct EmbeddedImage {
    pub archive_path: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub(crate) struct EpubPackageInput<'a> {
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub language: &'a str,
    pub mode: ExportContentMode,
    pub chapters: &'a [PreparedExportChapter],
    pub images: &'a HashMap<String, EmbeddedImage>,
    pub cover_src: Option<&'a str>,
    /// Maps an internal chapter `item_id` to its `chapter_NNN.xhtml` filename.
    pub item_to_xhtml: &'a HashMap<String, String>,
    /// Resolves an `epub://` archive link to an `item_id`, if any.
    pub resolve_book_link: &'a dyn Fn(&str) -> Option<String>,
}

const EPUB_CSS: &str = r#"body {
  font-family: -apple-system, BlinkMacSystemFont, "Georgia", serif;
  line-height: 1.65;
  color: #1c1c1e;
  margin: 1.2em;
}
h1, h2, h3, h4, h5, h6 {
  line-height: 1.28;
  margin-top: 1.4em;
  margin-bottom: 0.45em;
}
h1 { font-size: 1.75em; margin-top: 0.4em; }
h2 { font-size: 1.4em; }
h3 { font-size: 1.2em; }
.translated-title {
  font-size: 1.2em;
  font-weight: 600;
  color: #555558;
  margin-top: -0.2em;
  margin-bottom: 0.8em;
}
.byline {
  font-size: 0.88em;
  color: #636366;
  margin-bottom: 1.5em;
}
.byline a { color: #636366; }
.ai-summary {
  background: #f4f5f7;
  border-left: 3px solid #6e56cf;
  border-radius: 6px;
  padding: 0.85em 1.1em;
  margin: 1.2em 0 1.6em 0;
}
.ai-summary h2 {
  font-size: 0.95em;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: #555558;
  margin: 0 0 0.45em 0;
}
.ai-summary .tldr {
  font-weight: 600;
  margin: 0 0 0.5em 0;
}
.ai-summary ul {
  margin: 0;
  padding-left: 1.3em;
}
.translation, .cell-translation {
  color: #555558;
  margin-top: 0.25em;
}
p + p.translation {
  margin-top: -0.45em;
  margin-bottom: 1em;
}
.caption-translation {
  display: block;
  color: #636366;
  margin-top: 0.2em;
}
blockquote {
  border-left: 3px solid #007aff;
  margin: 0.9em 0;
  padding: 0.1em 0 0.1em 1em;
  color: #3a3a3c;
}
pre {
  background: #f2f2f7;
  border-radius: 6px;
  padding: 0.85em 1em;
  overflow-x: auto;
  font-family: ui-monospace, "SF Mono", Menlo, monospace;
  font-size: 0.88em;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}
code {
  font-family: ui-monospace, "SF Mono", Menlo, monospace;
  font-size: 0.9em;
  background: rgba(120, 120, 128, 0.12);
  padding: 0.1em 0.3em;
  border-radius: 3px;
}
pre code {
  background: transparent;
  padding: 0;
}
.tok-keyword { color: #ad3da4; }
.tok-string { color: #c41a16; }
.tok-comment { color: #6c7986; }
.tok-number, .tok-constant { color: #78492a; }
.tok-function { color: #0f68a0; }
.tok-type, .tok-tag { color: #1c464a; }
.tok-variable { color: #995500; }
.tok-attribute { color: #815f03; }
figure {
  margin: 1.3em 0;
  text-align: center;
}
figure img {
  max-width: 100%;
  height: auto;
}
figcaption {
  font-size: 0.86em;
  color: #636366;
  margin-top: 0.45em;
}
table {
  border-collapse: collapse;
  width: 100%;
  margin: 1.2em 0;
  font-size: 0.92em;
}
th, td {
  border: 1px solid #d1d1d6;
  padding: 0.45em 0.65em;
  text-align: left;
  vertical-align: top;
}
th {
  background: #f2f2f7;
  font-weight: 600;
}
.math-block {
  margin: 1em 0;
  text-align: center;
  overflow-x: auto;
}
hr {
  border: none;
  border-top: 1px solid #d1d1d6;
  margin: 1.8em auto;
  width: 40%;
}
"#;

fn is_valid_xml_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().filter(|&c| is_valid_xml_char(c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn distinct_translation<'a>(
    translations: &'a HashMap<String, String>,
    id: &str,
    original: &str,
) -> Option<&'a str> {
    let tr = translations.get(id)?.trim();
    if tr.is_empty() || tr == original.trim() {
        None
    } else {
        Some(tr)
    }
}

pub(crate) fn build_epub_archive(input: &EpubPackageInput<'_>) -> Result<Vec<u8>> {
    if input.chapters.is_empty() {
        return Err(EngineError::Invalid("没有可导出的章节内容。".into()));
    }

    let book_uuid = Uuid::new_v4();
    let lang = normalize_epub_lang(input.language);

    let mut chapter_files: Vec<(String, String, String, u32, bool)> = Vec::new();
    let mut xhtml_payloads: Vec<(String, String)> = Vec::new();

    for (idx, ch) in input.chapters.iter().enumerate() {
        let num = idx + 1;
        let cid = format!("chap_{num:03}");
        let fname = input
            .item_to_xhtml
            .get(&ch.item_id)
            .cloned()
            .unwrap_or_else(|| format!("chapter_{num:03}.xhtml"));
        let display_title = match input.mode {
            ExportContentMode::Translation => {
                distinct_translation(&ch.translations, "title", &ch.title)
                    .unwrap_or(&ch.title)
                    .to_string()
            }
            _ => ch.title.clone(),
        };
        let (body, has_math) = render_chapter_body(ch, input);
        let xhtml = wrap_xhtml(&display_title, &lang, &body);
        xhtml_payloads.push((fname.clone(), xhtml));
        chapter_files.push((cid, fname, display_title, ch.chapter_level, has_math));
    }

    let mut manifest_items = vec![
        r#"    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#
            .to_string(),
        r#"    <item id="css" href="style.css" media-type="text/css"/>"#.to_string(),
    ];
    let mut spine_items = Vec::new();
    for (cid, fname, _, _, has_math) in &chapter_files {
        let props = if *has_math {
            r#" properties="mathml""#
        } else {
            ""
        };
        manifest_items.push(format!(
            r#"    <item id="{cid}" href="{fname}" media-type="application/xhtml+xml"{props}/>"#
        ));
        spine_items.push(format!(r#"    <itemref idref="{cid}"/>"#));
    }

    let mut sorted_images: Vec<(&String, &EmbeddedImage)> = input.images.iter().collect();
    sorted_images.sort_by(|a, b| a.1.archive_path.cmp(&b.1.archive_path));

    let mut cover_assigned = false;
    for (idx, (orig_url, img)) in sorted_images.iter().enumerate() {
        let is_cover = !cover_assigned
            && input
                .cover_src
                .is_some_and(|c| c == orig_url.as_str());
        let props = if is_cover {
            cover_assigned = true;
            r#" properties="cover-image""#
        } else {
            ""
        };
        manifest_items.push(format!(
            r#"    <item id="img_{:04}" href="{}" media-type="{}"{props}/>"#,
            idx + 1,
            escape_xml(&img.archive_path),
            escape_xml(&img.mime),
        ));
    }

    let creator_xml = input
        .author
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| format!("\n    <dc:creator>{}</dc:creator>", escape_xml(a)))
        .unwrap_or_default();

    let modified = chrono_utc_timestamp();

    let opf_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:uuid:{book_uuid}</dc:identifier>
    <dc:title>{}</dc:title>
    <dc:language>{}</dc:language>{creator_xml}
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
{}
  </manifest>
  <spine>
{}
  </spine>
</package>
"#,
        escape_xml(input.title),
        escape_xml(&lang),
        manifest_items.join("\n"),
        spine_items.join("\n"),
    );

    let nav_ol = build_toc_ol(input, &chapter_files);
    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{}">
<head>
  <meta charset="utf-8"/>
  <title>{}</title>
  <link rel="stylesheet" type="text/css" href="style.css"/>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Contents</h1>
{nav_ol}
  </nav>
</body>
</html>
"#,
        escape_xml(&lang),
        escape_xml(input.title),
    );

    let container_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("mimetype", stored)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        zip.write_all(b"application/epub+zip")
            .map_err(|e| EngineError::Invalid(e.to_string()))?;

        zip.start_file("META-INF/container.xml", deflated)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        zip.write_all(container_xml.as_bytes())
            .map_err(|e| EngineError::Invalid(e.to_string()))?;

        zip.start_file("OEBPS/style.css", deflated)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        zip.write_all(EPUB_CSS.as_bytes())
            .map_err(|e| EngineError::Invalid(e.to_string()))?;

        zip.start_file("OEBPS/content.opf", deflated)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        zip.write_all(opf_xml.as_bytes())
            .map_err(|e| EngineError::Invalid(e.to_string()))?;

        zip.start_file("OEBPS/nav.xhtml", deflated)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        zip.write_all(nav_xhtml.as_bytes())
            .map_err(|e| EngineError::Invalid(e.to_string()))?;

        for (fname, content) in &xhtml_payloads {
            zip.start_file(format!("OEBPS/{fname}"), deflated)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
            zip.write_all(content.as_bytes())
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
        }

        for (_, img) in &sorted_images {
            zip.start_file(format!("OEBPS/{}", img.archive_path), stored)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
            zip.write_all(&img.bytes)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
        }

        zip.finish()
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
    }

    Ok(cursor.into_inner())
}

fn chrono_utc_timestamp() -> String {
    // ISO-8601 UTC timestamp without fractional seconds as required by EPUB 3.
    let secs = (eddy_model::now_ms() / 1000).max(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

fn normalize_epub_lang(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    if lower.contains("中文") || lower.contains("chinese") || lower == "cmn" || lower.starts_with("zh") {
        "zh-CN".into()
    } else if lower.contains("日本") || lower.contains("japanese") || lower == "jpn" || lower == "ja" {
        "ja".into()
    } else if lower.contains("한국") || lower.contains("korean") || lower == "kor" || lower == "ko" {
        "ko".into()
    } else if lower.contains("français") || lower.contains("french") || lower == "fra" || lower == "fr" {
        "fr".into()
    } else if lower.contains("deutsch") || lower.contains("german") || lower == "deu" || lower == "de" {
        "de".into()
    } else if lower.contains("español") || lower.contains("spanish") || lower == "spa" || lower == "es" {
        "es".into()
    } else {
        "en".into()
    }
}

fn wrap_xhtml(title: &str, lang: &str, body_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" lang="{}">
<head>
  <meta charset="utf-8"/>
  <title>{}</title>
  <link rel="stylesheet" type="text/css" href="style.css"/>
</head>
<body>
{body_inner}
</body>
</html>
"#,
        escape_xml(lang),
        escape_xml(title),
    )
}

fn build_toc_ol(
    input: &EpubPackageInput<'_>,
    chapter_files: &[(String, String, String, u32, bool)],
) -> String {
    if chapter_files.len() == 1 {
        let ch = &input.chapters[0];
        let fname = &chapter_files[0].1;
        let headings = extract_headings(&ch.document, &ch.translations, input.mode);
        if !headings.is_empty() {
            let entries: Vec<(String, String, usize)> = headings
                .into_iter()
                .map(|(id, text, level)| (format!("{fname}#{id}"), text, level))
                .collect();
            return render_nested_nav_ol(&entries);
        }
    }
    let entries: Vec<(String, String, usize)> = chapter_files
        .iter()
        .map(|(_, fname, title, level, _)| (fname.clone(), title.clone(), *level as usize))
        .collect();
    render_nested_nav_ol(&entries)
}

fn extract_headings(
    doc: &Document,
    translations: &HashMap<String, String>,
    mode: ExportContentMode,
) -> Vec<(String, String, usize)> {
    let mut raw = Vec::new();
    for block in &doc.blocks {
        if let BlockKind::Heading { level, runs } = &block.kind
            && *level <= 4
        {
            let orig = runs_plain_text(runs).trim().to_string();
            let text = match mode {
                ExportContentMode::Translation => distinct_translation(translations, &block.id, &orig)
                    .map(str::to_string)
                    .unwrap_or(orig),
                _ => orig,
            };
            if !text.is_empty() {
                raw.push((block.id.clone(), text, *level as usize));
            }
        }
    }
    let min_level = raw.iter().map(|(_, _, l)| *l).min().unwrap_or(1);
    raw.into_iter()
        .map(|(id, text, l)| (id, text, l.saturating_sub(min_level)))
        .collect()
}

/// Builds a strictly valid XHTML5 `<ol>` tree where child `<ol>` elements sit
/// inside their parent `<li>` element.
fn render_nested_nav_ol(entries: &[(String, String, usize)]) -> String {
    if entries.is_empty() {
        return "    <ol></ol>".into();
    }
    let mut out = String::new();
    out.push_str("    <ol>\n");
    let mut depth = 0usize;
    for (i, (href, label, raw_level)) in entries.iter().enumerate() {
        // Clamp jumps so a heading never skips more than 1 level deeper than current depth.
        let target = if i == 0 { 0 } else { (*raw_level).min(depth + 1) };
        if i > 0 {
            if target > depth {
                out.push_str("\n      <ol>\n");
                depth = target;
            } else {
                out.push_str("</li>\n");
                while depth > target {
                    out.push_str("      </ol>\n    </li>\n");
                    depth -= 1;
                }
            }
        }
        out.push_str(&format!(
            r#"      <li><a href="{}">{}</a>"#,
            escape_xml(href),
            escape_xml(label)
        ));
    }
    out.push_str("</li>\n");
    while depth > 0 {
        out.push_str("      </ol>\n    </li>\n");
        depth -= 1;
    }
    out.push_str("    </ol>");
    out
}

fn render_chapter_body(
    ch: &PreparedExportChapter,
    input: &EpubPackageInput<'_>,
) -> (String, bool) {
    let mut out = String::new();
    let mut has_math = false;

    // Header
    match input.mode {
        ExportContentMode::Original => {
            out.push_str(&format!("  <h1>{}</h1>\n", escape_xml(&ch.title)));
        }
        ExportContentMode::Translation => {
            let t = distinct_translation(&ch.translations, "title", &ch.title).unwrap_or(&ch.title);
            out.push_str(&format!("  <h1>{}</h1>\n", escape_xml(t)));
        }
        ExportContentMode::Bilingual => {
            out.push_str(&format!("  <h1>{}</h1>\n", escape_xml(&ch.title)));
            if let Some(t) = distinct_translation(&ch.translations, "title", &ch.title) {
                out.push_str(&format!(
                    "  <p class=\"translated-title\">{}</p>\n",
                    escape_xml(t)
                ));
            }
        }
    }

    let mut meta_parts = Vec::new();
    if let Some(author) = ch.author.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta_parts.push(escape_xml(author));
    }
    if let Some(source) = ch
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        meta_parts.push(escape_xml(source));
    }
    if let Some(url) = ch.url.as_deref().map(str::trim).filter(|u| u.starts_with("http")) {
        let esc = escape_xml(url);
        meta_parts.push(format!(r#"<a href="{esc}">Original</a>"#));
    }
    if !meta_parts.is_empty() {
        out.push_str(&format!(
            "  <p class=\"byline\">{}</p>\n",
            meta_parts.join(" · ")
        ));
    }

    if let Some(summary) = &ch.summary {
        out.push_str("  <aside class=\"ai-summary\">\n    <h2>Summary</h2>\n");
        out.push_str(&format!(
            "    <p class=\"tldr\">{}</p>\n",
            escape_xml(&summary.tldr)
        ));
        if !summary.points.is_empty() {
            out.push_str("    <ul>\n");
            for pt in &summary.points {
                out.push_str(&format!("      <li>{}</li>\n", escape_xml(pt)));
            }
            out.push_str("    </ul>\n");
        }
        out.push_str("  </aside>\n");
    }

    let mut quote_depth = 0u8;
    // Stack of open list types (`true` = `<ol>`, `false` = `<ul>`), each holding an open `<li>`.
    let mut list_stack: Vec<bool> = Vec::new();
    let mut li_open = false;

    let close_lists = |out: &mut String, stack: &mut Vec<bool>, li_open: &mut bool| {
        if *li_open {
            out.push_str("</li>\n");
            *li_open = false;
        }
        while let Some(ordered) = stack.pop() {
            out.push_str(if ordered { "  </ol>\n" } else { "  </ul>\n" });
            if !stack.is_empty() {
                out.push_str("  </li>\n");
            }
        }
    };

    for block in &ch.document.blocks {
        if block.quote_depth != quote_depth {
            close_lists(&mut out, &mut list_stack, &mut li_open);
            while quote_depth > block.quote_depth {
                out.push_str("  </blockquote>\n");
                quote_depth -= 1;
            }
            while quote_depth < block.quote_depth {
                out.push_str("  <blockquote>\n");
                quote_depth += 1;
            }
        }

        if let BlockKind::ListItem {
            ordered,
            depth,
            marker,
            runs,
            ..
        } = &block.kind
        {
            let target_len = (*depth as usize) + 1;
            if !*marker && list_stack.len() == target_len && li_open {
                // Continuation of the current list item without a new bullet.
                out.push_str("<br/>");
                let (inline_html, m) = render_text_runs_for_mode(
                    runs,
                    &ch.translations,
                    &block.id,
                    input,
                );
                has_math |= m;
                out.push_str(&inline_html);
                continue;
            }

            // Adjust list nesting to match `target_len` and `ordered`.
            while list_stack.len() > target_len {
                if li_open {
                    out.push_str("</li>\n");
                }
                let ord = list_stack.pop().unwrap();
                out.push_str(if ord { "  </ol>\n" } else { "  </ul>\n" });
                li_open = !list_stack.is_empty();
            }
            if list_stack.len() == target_len && list_stack.last() != Some(ordered) {
                if li_open {
                    out.push_str("</li>\n");
                }
                let ord = list_stack.pop().unwrap();
                out.push_str(if ord { "  </ol>\n" } else { "  </ul>\n" });
                li_open = !list_stack.is_empty();
            }
            while list_stack.len() < target_len {
                if list_stack.is_empty() {
                    out.push_str(if *ordered { "  <ol>\n" } else { "  <ul>\n" });
                } else {
                    if !li_open {
                        out.push_str("    <li>");
                    }
                    out.push('\n');
                    out.push_str(if *ordered { "    <ol>\n" } else { "    <ul>\n" });
                    li_open = false;
                }
                list_stack.push(*ordered);
            }
            if li_open {
                out.push_str("</li>\n");
            }
            out.push_str(&format!(r#"    <li id="{}">"#, escape_xml(&block.id)));
            let (inline_html, m) = render_text_runs_for_mode(
                runs,
                &ch.translations,
                &block.id,
                input,
            );
            has_math |= m;
            out.push_str(&inline_html);
            li_open = true;
            continue;
        }

        close_lists(&mut out, &mut list_stack, &mut li_open);
        let (block_html, m) = render_non_list_block(block, &ch.translations, input);
        has_math |= m;
        out.push_str(&block_html);
    }

    close_lists(&mut out, &mut list_stack, &mut li_open);
    while quote_depth > 0 {
        out.push_str("  </blockquote>\n");
        quote_depth -= 1;
    }

    (out, has_math)
}

fn render_text_runs_for_mode(
    runs: &[Run],
    translations: &HashMap<String, String>,
    seg_id: &str,
    input: &EpubPackageInput<'_>,
) -> (String, bool) {
    let orig_plain = runs_plain_text(runs);
    match input.mode {
        ExportContentMode::Original => render_runs(runs, input),
        ExportContentMode::Translation => {
            if let Some(tr) = distinct_translation(translations, seg_id, &orig_plain) {
                let tr_runs = translated_runs(tr);
                render_runs(&tr_runs, input)
            } else {
                render_runs(runs, input)
            }
        }
        ExportContentMode::Bilingual => {
            let (mut html, mut math) = render_runs(runs, input);
            if let Some(tr) = distinct_translation(translations, seg_id, &orig_plain) {
                let tr_runs = translated_runs(tr);
                let (tr_html, m2) = render_runs(&tr_runs, input);
                math |= m2;
                html.push_str(&format!(r#"<div class="translation">{tr_html}</div>"#));
            }
            (html, math)
        }
    }
}

fn render_non_list_block(
    block: &Block,
    translations: &HashMap<String, String>,
    input: &EpubPackageInput<'_>,
) -> (String, bool) {
    let id_attr = escape_xml(&block.id);
    match &block.kind {
        BlockKind::Heading { level, runs } => {
            let tag_level = (*level).clamp(1, 5) + 1; // h2..h6 under chapter h1
            let orig_plain = runs_plain_text(runs);
            match input.mode {
                ExportContentMode::Original => {
                    let (inner, m) = render_runs(runs, input);
                    (format!("  <h{tag_level} id=\"{id_attr}\">{inner}</h{tag_level}>\n"), m)
                }
                ExportContentMode::Translation => {
                    if let Some(tr) = distinct_translation(translations, &block.id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let (inner, m) = render_runs(&tr_runs, input);
                        (format!("  <h{tag_level} id=\"{id_attr}\">{inner}</h{tag_level}>\n"), m)
                    } else {
                        let (inner, m) = render_runs(runs, input);
                        (format!("  <h{tag_level} id=\"{id_attr}\">{inner}</h{tag_level}>\n"), m)
                    }
                }
                ExportContentMode::Bilingual => {
                    let (mut inner, mut m) = render_runs(runs, input);
                    let mut html =
                        format!("  <h{tag_level} id=\"{id_attr}\">{inner}</h{tag_level}>\n");
                    if let Some(tr) = distinct_translation(translations, &block.id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let m2;
                        (inner, m2) = render_runs(&tr_runs, input);
                        m |= m2;
                        html.push_str(&format!("  <p class=\"translation\">{inner}</p>\n"));
                    }
                    (html, m)
                }
            }
        }
        BlockKind::Paragraph { runs } => {
            let orig_plain = runs_plain_text(runs);
            match input.mode {
                ExportContentMode::Original => {
                    let (inner, m) = render_runs(runs, input);
                    (format!("  <p id=\"{id_attr}\">{inner}</p>\n"), m)
                }
                ExportContentMode::Translation => {
                    if let Some(tr) = distinct_translation(translations, &block.id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let (inner, m) = render_runs(&tr_runs, input);
                        (format!("  <p id=\"{id_attr}\">{inner}</p>\n"), m)
                    } else {
                        let (inner, m) = render_runs(runs, input);
                        (format!("  <p id=\"{id_attr}\">{inner}</p>\n"), m)
                    }
                }
                ExportContentMode::Bilingual => {
                    let (orig_html, mut m) = render_runs(runs, input);
                    let mut html = format!("  <p id=\"{id_attr}\">{orig_html}</p>\n");
                    if let Some(tr) = distinct_translation(translations, &block.id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let (tr_html, m2) = render_runs(&tr_runs, input);
                        m |= m2;
                        html.push_str(&format!("  <p class=\"translation\">{tr_html}</p>\n"));
                    }
                    (html, m)
                }
            }
        }
        BlockKind::Code {
            language,
            text,
            highlights,
        } => {
            let cls = language
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| format!(r#" class="language-{}""#, escape_xml(l)))
                .unwrap_or_default();
            let highlighted = render_highlighted_code(text, highlights);
            (
                format!("  <pre id=\"{id_attr}\"><code{cls}>{highlighted}</code></pre>\n"),
                false,
            )
        }
        BlockKind::Math { tex } => {
            let trimmed = tex.trim();
            if trimmed.is_empty() {
                return (String::new(), false);
            }
            let esc = escape_xml(trimmed);
            (
                format!(
                    r#"  <div id="{id_attr}" class="math-block"><math xmlns="http://www.w3.org/1998/Math/MathML" display="block" alttext="{esc}"><semantics><mrow><mtext>{esc}</mtext></mrow><annotation encoding="application/x-tex">{esc}</annotation></semantics></math></div>"#
                ) + "\n",
                true,
            )
        }
        BlockKind::Image {
            src, alt, caption, ..
        } => {
            let mut m = false;
            let cap_id = format!("{}#caption", block.id);
            let figcap = if !caption.is_empty() {
                let orig_cap = runs_plain_text(caption);
                let cap_inner = match input.mode {
                    ExportContentMode::Original => {
                        let (h, m1) = render_runs(caption, input);
                        m |= m1;
                        h
                    }
                    ExportContentMode::Translation => {
                        if let Some(tr) = distinct_translation(translations, &cap_id, &orig_cap) {
                            let tr_runs = translated_runs(tr);
                            let (h, m1) = render_runs(&tr_runs, input);
                            m |= m1;
                            h
                        } else {
                            let (h, m1) = render_runs(caption, input);
                            m |= m1;
                            h
                        }
                    }
                    ExportContentMode::Bilingual => {
                        let (mut h, m1) = render_runs(caption, input);
                        m |= m1;
                        if let Some(tr) = distinct_translation(translations, &cap_id, &orig_cap) {
                            let tr_runs = translated_runs(tr);
                            let (th, m2) = render_runs(&tr_runs, input);
                            m |= m2;
                            h.push_str(&format!(
                                r#"<span class="caption-translation">{th}</span>"#
                            ));
                        }
                        h
                    }
                };
                format!("<figcaption>{cap_inner}</figcaption>")
            } else {
                String::new()
            };

            if let Some(img) = input.images.get(src) {
                (
                    format!(
                        "  <figure id=\"{id_attr}\"><img src=\"{}\" alt=\"{}\"/>{figcap}</figure>\n",
                        escape_xml(&img.archive_path),
                        escape_xml(alt),
                    ),
                    m,
                )
            } else if !figcap.is_empty() {
                (format!("  <figure id=\"{id_attr}\">{figcap}</figure>\n"), m)
            } else {
                (String::new(), false)
            }
        }
        BlockKind::Table { rows } => render_table(&id_attr, &block.id, rows, translations, input),
        BlockKind::Media {
            media, src, title, ..
        } => {
            let label = title.as_deref().unwrap_or(match media {
                MediaKind::Audio => "Audio",
                MediaKind::Video => "Video",
                MediaKind::Embed => "Embedded media",
            });
            if src.starts_with("http://") || src.starts_with("https://") {
                let esc_src = escape_xml(src);
                (
                    format!(
                        "  <p id=\"{id_attr}\"><a href=\"{esc_src}\">{}</a></p>\n",
                        escape_xml(label)
                    ),
                    false,
                )
            } else {
                (
                    format!("  <p id=\"{id_attr}\">{}</p>\n", escape_xml(label)),
                    false,
                )
            }
        }
        BlockKind::Rule => (format!("  <hr id=\"{id_attr}\"/>\n"), false),
        BlockKind::ListItem { .. } => unreachable!("handled in list loop"),
    }
}

fn render_table(
    id_attr: &str,
    block_id: &str,
    rows: &[TableRow],
    translations: &HashMap<String, String>,
    input: &EpubPackageInput<'_>,
) -> (String, bool) {
    if rows.is_empty() {
        return (String::new(), false);
    }
    let mut m = false;
    let mut out = format!("  <table id=\"{id_attr}\">\n    <tbody>\n");
    for (r_idx, row) in rows.iter().enumerate() {
        out.push_str("      <tr>");
        for (c_idx, cell) in row.cells.iter().enumerate() {
            let tag = if cell.header { "th" } else { "td" };
            let mut attrs = String::new();
            if cell.colspan > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
            }
            if cell.rowspan > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
            }
            let seg_id = format!("{block_id}#r{r_idx}c{c_idx}");
            let orig_plain = runs_plain_text(&cell.runs);
            let cell_html = match input.mode {
                ExportContentMode::Original => {
                    let (h, m1) = render_runs(&cell.runs, input);
                    m |= m1;
                    h
                }
                ExportContentMode::Translation => {
                    if let Some(tr) = distinct_translation(translations, &seg_id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let (h, m1) = render_runs(&tr_runs, input);
                        m |= m1;
                        h
                    } else {
                        let (h, m1) = render_runs(&cell.runs, input);
                        m |= m1;
                        h
                    }
                }
                ExportContentMode::Bilingual => {
                    let (mut h, m1) = render_runs(&cell.runs, input);
                    m |= m1;
                    if let Some(tr) = distinct_translation(translations, &seg_id, &orig_plain) {
                        let tr_runs = translated_runs(tr);
                        let (th, m2) = render_runs(&tr_runs, input);
                        m |= m2;
                        h.push_str(&format!(r#"<div class="cell-translation">{th}</div>"#));
                    }
                    h
                }
            };
            out.push_str(&format!("<{tag}{attrs}>{cell_html}</{tag}>"));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("    </tbody>\n  </table>\n");
    (out, m)
}

fn render_runs(runs: &[Run], input: &EpubPackageInput<'_>) -> (String, bool) {
    let mut out = String::new();
    let mut has_math = false;
    for run in runs {
        if run.math {
            let trimmed = run.text.trim();
            if !trimmed.is_empty() {
                has_math = true;
                let esc = escape_xml(trimmed);
                out.push_str(&format!(
                    r#"<math xmlns="http://www.w3.org/1998/Math/MathML" alttext="{esc}"><semantics><mrow><mtext>{esc}</mtext></mrow><annotation encoding="application/x-tex">{esc}</annotation></semantics></math>"#
                ));
            }
            continue;
        }
        let mut s = escape_xml(&run.text);
        if s.is_empty() {
            continue;
        }
        if run.style & style::CODE != 0 {
            s = format!("<code>{s}</code>");
        }
        if run.style & style::BOLD != 0 {
            s = format!("<strong>{s}</strong>");
        }
        if run.style & style::ITALIC != 0 {
            s = format!("<em>{s}</em>");
        }
        if run.style & style::STRIKE != 0 {
            s = format!("<del>{s}</del>");
        }
        if run.style & style::UNDERLINE != 0 {
            s = format!("<u>{s}</u>");
        }
        if run.style & style::SUPERSCRIPT != 0 {
            s = format!("<sup>{s}</sup>");
        }
        if run.style & style::SUBSCRIPT != 0 {
            s = format!("<sub>{s}</sub>");
        }
        if run.style & style::SMALL != 0 {
            s = format!("<small>{s}</small>");
        }
        if run.style & style::MARK != 0 {
            s = format!("<mark>{s}</mark>");
        }
        if let Some(link) = &run.link
            && let Some(href) = resolve_href(link, input)
        {
            s = format!(r#"<a href="{}">{s}</a>"#, escape_xml(&href));
        }
        out.push_str(&s);
    }
    (out, has_math)
}

fn resolve_href(raw: &str, input: &EpubPackageInput<'_>) -> Option<String> {
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("mailto:") {
        return Some(raw.to_string());
    }
    if raw.starts_with("epub://")
        && let Some(item_id) = (input.resolve_book_link)(raw)
        && let Some(fname) = input.item_to_xhtml.get(&item_id)
    {
        return Some(fname.clone());
    }
    None
}

fn render_highlighted_code(text: &str, highlights: &[HighlightSpan]) -> String {
    if highlights.is_empty() {
        return escape_xml(text);
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let len = units.len();
    let mut out = String::with_capacity(text.len() + highlights.len() * 24);
    let mut cursor = 0usize;
    for span in highlights {
        let start = (span.start as usize).min(len);
        let end = (span.end as usize).min(len);
        if start < cursor || start >= end {
            continue;
        }
        if start > cursor {
            out.push_str(&escape_xml(&String::from_utf16_lossy(&units[cursor..start])));
        }
        let class = match span.class {
            TokenClass::Keyword => "tok-keyword",
            TokenClass::String => "tok-string",
            TokenClass::Comment => "tok-comment",
            TokenClass::Number => "tok-number",
            TokenClass::Constant => "tok-constant",
            TokenClass::Function => "tok-function",
            TokenClass::Type => "tok-type",
            TokenClass::Variable => "tok-variable",
            TokenClass::Operator => "tok-operator",
            TokenClass::Punctuation => "tok-punctuation",
            TokenClass::Tag => "tok-tag",
            TokenClass::Attribute => "tok-attribute",
        };
        let piece = escape_xml(&String::from_utf16_lossy(&units[start..end]));
        out.push_str(&format!(r#"<span class="{class}">{piece}</span>"#));
        cursor = end;
    }
    if cursor < len {
        out.push_str(&escape_xml(&String::from_utf16_lossy(&units[cursor..])));
    }
    out
}
