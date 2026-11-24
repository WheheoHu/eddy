use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use uuid::Uuid;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::VideoError;
use crate::models::{ArticleBlock, ArticleDocument, ArticleSection, format_timestamp_ms};

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
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

pub fn make_jump_url(source_url: Option<&str>, timestamp_ms: u64) -> Option<String> {
    let url = source_url?.trim();
    if url.is_empty() {
        return None;
    }
    let sec = (timestamp_ms + 500) / 1000;
    let sep = if url.contains('?') { '&' } else { '?' };
    if url.contains("youtube.com/watch") {
        Some(format!("{url}{sep}t={sec}s"))
    } else if url.contains("youtu.be/") || url.contains("bilibili.com/video/") {
        Some(format!("{url}{sep}t={sec}"))
    } else {
        Some(url.to_string())
    }
}

/// Builds a standard multi-chapter EPUB 3 archive in memory (`chapter_00.xhtml` overview + one file per chapter)
/// so Eddy's `import_epub` creates a complete `Series` with per-chapter TOC entries.
pub fn build_epub_bytes(doc: &ArticleDocument) -> Result<(Vec<u8>, u32), VideoError> {
    let is_zh = doc
        .title
        .chars()
        .chain(doc.executive_summary.chars())
        .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    let lang = if is_zh { "zh" } else { "en" };
    let book_uuid = Uuid::new_v4();

    // Collect unique existing slide images
    let mut image_order: Vec<(String, PathBuf)> = Vec::new();
    let mut image_map: HashMap<String, String> = HashMap::new();
    for sec in &doc.sections {
        for blk in &sec.blocks {
            if blk.kind == "figure"
                && let Some(ref p_str) = blk.image_path
                && !image_map.contains_key(p_str)
            {
                let p = PathBuf::from(p_str);
                if p.is_file() {
                    let idx = image_order.len() + 1;
                    let arc_rel = format!("images/slide_{idx:03}.jpg");
                    image_map.insert(p_str.clone(), arc_rel.clone());
                    image_order.push((arc_rel, p));
                }
            }
        }
    }

    let mut chapter_files: Vec<(String, String, String)> = Vec::new();
    let mut xhtml_payloads: Vec<(String, String)> = Vec::new();

    if doc.sections.len() > 1 {
        if !doc.executive_summary.trim().is_empty() || !doc.key_takeaways.is_empty() {
            let ov_title = if is_zh {
                "导读与核心要点"
            } else {
                "Executive Summary"
            };
            let ov_body = build_overview_xhtml(doc, is_zh, true);
            let fname = "chapter_00.xhtml".to_string();
            xhtml_payloads.push((fname.clone(), wrap_xhtml(ov_title, lang, &ov_body)));
            chapter_files.push(("chap_00".into(), fname, ov_title.into()));
        }
        for sec in &doc.sections {
            let cid = format!("chap_{:02}", sec.index);
            let fname = format!("chapter_{:02}.xhtml", sec.index);
            let body = build_section_xhtml(doc, sec, &image_map, false);
            xhtml_payloads.push((fname.clone(), wrap_xhtml(&sec.title, lang, &body)));
            chapter_files.push((cid, fname, sec.title.clone()));
        }
    } else {
        let fname = "article.xhtml".to_string();
        let mut combined = Vec::new();
        let ov = build_overview_xhtml(doc, is_zh, false);
        if !ov.is_empty() {
            combined.push(ov);
            combined.push("<hr/>".into());
        }
        for sec in &doc.sections {
            combined.push(build_section_xhtml(doc, sec, &image_map, true));
        }
        xhtml_payloads.push((
            fname.clone(),
            wrap_xhtml(&doc.title, lang, &combined.join("\n")),
        ));
        chapter_files.push(("chap_01".into(), fname, doc.title.clone()));
    }

    let mut manifest_items = vec![
        r#"    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#
            .to_string(),
    ];
    let mut spine_items = Vec::new();
    for (cid, fname, _) in &chapter_files {
        manifest_items.push(format!(
            r#"    <item id="{cid}" href="{fname}" media-type="application/xhtml+xml"/>"#
        ));
        spine_items.push(format!(r#"    <itemref idref="{cid}"/>"#));
    }
    for (idx, (arc_rel, _)) in image_order.iter().enumerate() {
        let props = if idx == 0 {
            r#" properties="cover-image""#
        } else {
            ""
        };
        manifest_items.push(format!(
            r#"    <item id="img_{:03}" href="{arc_rel}" media-type="image/jpeg"{props}/>"#,
            idx + 1
        ));
    }

    let creator_xml = doc
        .author
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .map(|a| format!("<dc:creator>{}</dc:creator>", escape_xml(a)))
        .unwrap_or_default();

    let opf_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:uuid:{book_uuid}</dc:identifier>
    <dc:title>{}</dc:title>
    <dc:language>{lang}</dc:language>
    {creator_xml}
  </metadata>
  <manifest>
{}
  </manifest>
  <spine>
{}
  </spine>
</package>
"#,
        escape_xml(&doc.title),
        manifest_items.join("\n"),
        spine_items.join("\n")
    );

    let nav_lis: String = chapter_files
        .iter()
        .map(|(_, fname, ctitle)| {
            format!(
                r#"        <li><a href="{fname}">{}</a></li>"#,
                escape_xml(ctitle)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{lang}">
<head><title>{}</title></head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Contents</h1>
    <ol>
{nav_lis}
    </ol>
  </nav>
</body>
</html>
"#,
        escape_xml(&doc.title)
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
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("mimetype", stored)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        zip.write_all(b"application/epub+zip")?;

        zip.start_file("META-INF/container.xml", deflated)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        zip.write_all(container_xml.as_bytes())?;

        zip.start_file("OEBPS/content.opf", deflated)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        zip.write_all(opf_xml.as_bytes())?;

        zip.start_file("OEBPS/nav.xhtml", deflated)
            .map_err(|e| VideoError::Format(e.to_string()))?;
        zip.write_all(nav_xhtml.as_bytes())?;

        for (fname, content) in &xhtml_payloads {
            zip.start_file(format!("OEBPS/{fname}"), deflated)
                .map_err(|e| VideoError::Format(e.to_string()))?;
            zip.write_all(content.as_bytes())?;
        }

        for (arc_rel, src_path) in &image_order {
            if let Ok(img_bytes) = std::fs::read(src_path) {
                zip.start_file(format!("OEBPS/{arc_rel}"), stored)
                    .map_err(|e| VideoError::Format(e.to_string()))?;
                zip.write_all(&img_bytes)?;
            }
        }
        zip.finish()
            .map_err(|e| VideoError::Format(e.to_string()))?;
    }

    let chapters_count = chapter_files.len() as u32;
    Ok((cursor.into_inner(), chapters_count))
}

fn wrap_xhtml(title: &str, lang: &str, body_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" lang="{lang}">
<head>
  <meta charset="utf-8"/>
  <title>{}</title>
</head>
<body>
{body_inner}
</body>
</html>
"#,
        escape_xml(title)
    )
}

/// Formats a single takeaway string as safe XHTML, bolding a concise leading topic label
/// (`Topic: detail` or `主题：说明`) and stripping any stray Markdown `**...**` markers.
pub(crate) fn format_takeaway_html(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Handle leading Markdown bold like `**Topic**: detail` or `**Topic**：detail`
    if let Some(after_open) = trimmed.strip_prefix("**")
        && let Some(close_idx) = after_open.find("**")
    {
        let label = after_open[..close_idx].trim();
        let rest = after_open[close_idx + 2..].trim_start();
        if !label.is_empty() && label.chars().count() <= 40 {
            if let Some(detail) = rest.strip_prefix('：').map(str::trim)
                && !detail.is_empty()
            {
                return format!(
                    "<strong>{}：</strong>{}",
                    escape_xml(label),
                    escape_xml(detail)
                );
            }
            if let Some(detail) = rest.strip_prefix(':').map(str::trim)
                && !detail.is_empty()
            {
                return format!(
                    "<strong>{}: </strong>{}",
                    escape_xml(label),
                    escape_xml(detail)
                );
            }
            if !rest.is_empty() {
                return format!(
                    "<strong>{}</strong> {}",
                    escape_xml(label),
                    escape_xml(rest)
                );
            }
            return format!("<strong>{}</strong>", escape_xml(label));
        }
    }

    // Split on full-width `：` or ASCII `: ` when the leading label looks like a concise topic.
    let split = trimmed
        .find('：')
        .map(|idx| (idx, "：", idx + '：'.len_utf8()))
        .into_iter()
        .chain(trimmed.find(": ").map(|idx| (idx, ": ", idx + 2)))
        .min_by_key(|(idx, _, _)| *idx);

    if let Some((sep_start, sep, detail_start)) = split {
        let label = trimmed[..sep_start].trim();
        let detail = trimmed[detail_start..].trim();
        let label_len = label.chars().count();
        let lower_label = label.to_ascii_lowercase();
        let is_url_scheme = lower_label.ends_with("http") || lower_label.ends_with("https");
        let is_timestamp =
            !label.is_empty() && label.chars().all(|c| c.is_ascii_digit() || c == ':');
        if (1..=40).contains(&label_len) && !detail.is_empty() && !is_url_scheme && !is_timestamp {
            return format!(
                "<strong>{}{sep}</strong>{}",
                escape_xml(label),
                escape_xml(detail)
            );
        }
    }

    escape_xml(trimmed)
}

fn build_overview_xhtml(doc: &ArticleDocument, is_zh: bool, include_chapter_guide: bool) -> String {
    let mut parts = Vec::new();
    if !doc.subtitle.trim().is_empty() {
        parts.push(format!(
            "<p><em>{}</em></p>",
            escape_xml(doc.subtitle.trim())
        ));
    }

    let mut meta_items = Vec::new();
    if let Some(url) = doc.source_url.as_deref().filter(|u| !u.trim().is_empty()) {
        let esc = escape_xml(url.trim());
        if is_zh {
            meta_items.push(format!(r#"原视频：<a href="{esc}">观看原片</a>"#));
        } else {
            meta_items.push(format!(
                r#"Original Video: <a href="{esc}">Watch source</a>"#
            ));
        }
    }
    if doc.duration_ms > 0 {
        let dur = format_timestamp_ms(doc.duration_ms);
        if is_zh {
            meta_items.push(format!("时长 {dur}"));
        } else {
            meta_items.push(format!("Duration {dur}"));
        }
    }
    if include_chapter_guide && doc.sections.len() > 1 {
        if is_zh {
            meta_items.push(format!("共 {} 章", doc.sections.len()));
        } else {
            meta_items.push(format!("{} chapters", doc.sections.len()));
        }
    }
    if !meta_items.is_empty() {
        parts.push(format!("<p><small>{}</small></p>", meta_items.join(" · ")));
    }

    let summary_paras: Vec<&str> = doc
        .executive_summary
        .trim()
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if !summary_paras.is_empty() {
        parts.push(if is_zh {
            "<h3>核心概览</h3>".into()
        } else {
            "<h3>Overview</h3>".into()
        });
        let inner: String = summary_paras
            .into_iter()
            .map(|p| format!("<p>{}</p>", escape_xml(p)))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("<blockquote>\n{inner}\n</blockquote>"));
    }

    let takeaway_lis: String = doc
        .key_takeaways
        .iter()
        .map(|t| format_takeaway_html(t))
        .filter(|h| !h.is_empty())
        .map(|h| format!("<li>{h}</li>"))
        .collect();
    if !takeaway_lis.is_empty() {
        parts.push(if is_zh {
            "<h3>核心要点</h3>".into()
        } else {
            "<h3>Key Takeaways</h3>".into()
        });
        parts.push(format!("<ul>{takeaway_lis}</ul>"));
    }

    if include_chapter_guide && doc.sections.len() > 1 {
        let guide_lis: String = doc
            .sections
            .iter()
            .map(|sec| {
                let fname = format!("chapter_{:02}.xhtml", sec.index);
                let ts_range = format!(
                    "{}–{}",
                    format_timestamp_ms(sec.start_ms),
                    format_timestamp_ms(sec.end_ms)
                );
                let brief = sec.brief.trim();
                let brief_suffix = if brief.is_empty() {
                    String::new()
                } else if is_zh {
                    format!("：{}", escape_xml(brief))
                } else {
                    format!(" — {}", escape_xml(brief))
                };
                format!(
                    r#"<li><strong><a href="{fname}">{}</a></strong> <small>({ts_range})</small>{brief_suffix}</li>"#,
                    escape_xml(&sec.title)
                )
            })
            .collect();
        if !guide_lis.is_empty() {
            parts.push(if is_zh {
                "<h3>章节导览</h3>".into()
            } else {
                "<h3>Chapter Guide</h3>".into()
            });
            parts.push(format!("<ol>{guide_lis}</ol>"));
        }
    }

    parts.join("\n")
}

fn build_section_xhtml(
    doc: &ArticleDocument,
    sec: &ArticleSection,
    image_map: &HashMap<String, String>,
    emit_heading: bool,
) -> String {
    let mut parts = Vec::new();
    let ts_range = format!(
        "{}–{}",
        format_timestamp_ms(sec.start_ms),
        format_timestamp_ms(sec.end_ms)
    );
    let jump = make_jump_url(doc.source_url.as_deref(), sec.start_ms);

    if emit_heading {
        if let Some(u) = jump {
            parts.push(format!(
                r#"<h2>{}. {} (<a href="{}">{ts_range}</a>)</h2>"#,
                sec.index,
                escape_xml(&sec.title),
                escape_xml(&u)
            ));
        } else {
            parts.push(format!(
                "<h2>{}. {} ({ts_range})</h2>",
                sec.index,
                escape_xml(&sec.title)
            ));
        }
    } else if let Some(u) = jump {
        parts.push(format!(
            r#"<p><small>Video Segment: <a href="{}">{ts_range}</a></small></p>"#,
            escape_xml(&u)
        ));
    } else {
        parts.push(format!("<p><small>Video Segment: {ts_range}</small></p>"));
    }

    for blk in &sec.blocks {
        let html = block_to_xhtml(blk, doc.source_url.as_deref(), image_map);
        if !html.is_empty() {
            parts.push(html);
        }
    }
    parts.join("\n")
}

fn block_to_xhtml(
    blk: &ArticleBlock,
    source_url: Option<&str>,
    image_map: &HashMap<String, String>,
) -> String {
    match blk.kind.as_str() {
        "paragraph" if !blk.text.trim().is_empty() => {
            format!("<p>{}</p>", escape_xml(blk.text.trim()))
        }
        "subheading" if !blk.text.trim().is_empty() => {
            format!("<h3>{}</h3>", escape_xml(blk.text.trim()))
        }
        "bullet_list" if !blk.items.is_empty() => {
            let lis: String = blk
                .items
                .iter()
                .filter(|x| !x.trim().is_empty())
                .map(|x| format!("<li>{}</li>", escape_xml(x.trim())))
                .collect();
            format!("<ul>{lis}</ul>")
        }
        "numbered_list" if !blk.items.is_empty() => {
            let lis: String = blk
                .items
                .iter()
                .filter(|x| !x.trim().is_empty())
                .map(|x| format!("<li>{}</li>", escape_xml(x.trim())))
                .collect();
            format!("<ol>{lis}</ol>")
        }
        "quote" if !blk.text.trim().is_empty() => {
            format!(
                "<blockquote><p>{}</p></blockquote>",
                escape_xml(blk.text.trim())
            )
        }
        "code" if !blk.text.trim().is_empty() => {
            let cls = blk
                .language
                .as_deref()
                .filter(|l| !l.trim().is_empty())
                .map(|l| format!(r#" class="language-{}""#, escape_xml(l.trim())))
                .unwrap_or_default();
            format!(
                "<pre><code{cls}>{}</code></pre>",
                escape_xml(blk.text.trim_end_matches('\n'))
            )
        }
        "math" => {
            let tex = blk
                .tex
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(blk.text.trim());
            if tex.is_empty() {
                String::new()
            } else {
                let esc = escape_xml(tex);
                format!(
                    r#"<math xmlns="http://www.w3.org/1998/Math/MathML" display="block" alttext="{esc}"><semantics><annotation encoding="application/x-tex">{esc}</annotation></semantics></math>"#
                )
            }
        }
        "table" if !blk.headers.is_empty() || !blk.rows.is_empty() => {
            let thead = if blk.headers.is_empty() {
                String::new()
            } else {
                let ths: String = blk
                    .headers
                    .iter()
                    .map(|h| format!("<th>{}</th>", escape_xml(h.trim())))
                    .collect();
                format!("<thead><tr>{ths}</tr></thead>")
            };
            let trs: String = blk
                .rows
                .iter()
                .map(|row| {
                    let tds: String = row
                        .iter()
                        .map(|c| format!("<td>{}</td>", escape_xml(c.trim())))
                        .collect();
                    format!("<tr>{tds}</tr>")
                })
                .collect();
            format!("<table>{thead}<tbody>{trs}</tbody></table>")
        }
        "figure" => {
            let Some(ref path_str) = blk.image_path else {
                return String::new();
            };
            let Some(arc_rel) = image_map.get(path_str) else {
                return String::new();
            };
            if !Path::new(path_str).is_file() {
                return String::new();
            }
            let mut cap_pieces = Vec::new();
            if let Some(ts_ms) = blk.timestamp_ms {
                let ts_str = format_timestamp_ms(ts_ms);
                if let Some(u) = make_jump_url(source_url, ts_ms) {
                    cap_pieces.push(format!(r#"<a href="{}">[{ts_str}]</a>"#, escape_xml(&u)));
                } else {
                    cap_pieces.push(format!("[{ts_str}]"));
                }
            }
            if !blk.caption.trim().is_empty() {
                cap_pieces.push(escape_xml(blk.caption.trim()));
            }
            let figcap = if cap_pieces.is_empty() {
                String::new()
            } else {
                format!("<figcaption>{}</figcaption>", cap_pieces.join(" "))
            };
            format!(
                r#"<figure><img src="{arc_rel}" alt="{}"/>{figcap}</figure>"#,
                escape_xml(blk.caption.trim())
            )
        }
        _ => String::new(),
    }
}
