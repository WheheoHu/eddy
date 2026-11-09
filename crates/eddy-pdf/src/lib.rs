//! Reads PDF text with pdfium, statically linked (see
//! `scripts/fetch-pdfium.sh`). Characters are grouped into visual lines with
//! their boxes and font sizes; `eddy_core` turns those into a reflowed
//! document. Nothing here knows about columns, headers or headings.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use eddy_core::{PdfLine, PdfReader, PdfText};
use pdfium_render::prelude::*;

/// `PdfReader` backed by pdfium. Cheap to construct; the library is
/// initialised once per process on first use.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfiumReader;

impl PdfiumReader {
    pub fn new() -> Self {
        Self
    }

    /// Reads the PDF at `path`; `None` when pdfium cannot open it.
    pub fn read_file(&self, path: &Path) -> Option<PdfText> {
        let pdfium = pdfium()?;
        let document = pdfium.load_pdf_from_file(path, None).ok()?;
        Some(read_document(&document))
    }
}

impl PdfReader for PdfiumReader {
    fn read(&self, path: &Path) -> Option<PdfText> {
        self.read_file(path)
    }
}

fn pdfium() -> Option<&'static Pdfium> {
    static PDFIUM: OnceLock<Option<Pdfium>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            Pdfium::bind_to_statically_linked_library()
                .ok()
                .map(Pdfium::new)
        })
        .as_ref()
}

fn read_document(document: &PdfDocument<'_>) -> PdfText {
    let metadata = document.metadata();
    let tag = |t: PdfDocumentMetadataTagType| {
        metadata
            .get(t)
            .map(|v| v.value().trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let pages = document
        .pages()
        .iter()
        .map(|page| read_page(&page))
        .collect();
    PdfText {
        title: tag(PdfDocumentMetadataTagType::Title),
        author: tag(PdfDocumentMetadataTagType::Author),
        pages,
    }
}

fn read_page(page: &PdfPage<'_>) -> eddy_core::PdfPage {
    let width = page.width().value;
    let height = page.height().value;
    let lines = match page.text() {
        Ok(text) => LineBuilder::default().run(&text),
        Err(_) => Vec::new(),
    };
    eddy_core::PdfPage {
        width,
        height,
        lines,
    }
}

/// One character with what the line grouping needs.
struct Glyph {
    ch: char,
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    baseline: f32,
    font_size: f32,
    bold: bool,
}

#[derive(Default)]
struct LineBuilder {
    lines: Vec<PdfLine>,
    current: Vec<Glyph>,
}

impl LineBuilder {
    fn run(mut self, text: &PdfPageText<'_>) -> Vec<PdfLine> {
        let chars = text.chars();
        for c in chars.iter() {
            let Some(mut ch) = c.unicode_char() else { continue };
            if ch == '\r' || ch == '\n' {
                // pdfium marks line breaks it inferred; geometry decides
                // for us, but a break is still a hard boundary.
                self.flush();
                continue;
            }
            if ch.is_control() {
                // Fonts without a Unicode map (TeX's Type 1 fonts) give
                // hyphens a control code; pdfium still knows what they are.
                if c.is_hyphen().unwrap_or(false) {
                    ch = '-';
                } else {
                    continue;
                }
            }
            let Ok(bounds) = c.loose_bounds() else { continue };
            let Ok((_, baseline)) = c.origin() else { continue };
            let font_size = c.scaled_font_size().value.abs();
            if font_size <= 0.0 {
                continue;
            }
            let glyph = Glyph {
                ch,
                left: bounds.left().value,
                right: bounds.right().value,
                bottom: bounds.bottom().value,
                top: bounds.top().value,
                baseline: baseline.value,
                font_size,
                bold: is_bold(&c),
            };
            if !self.continues(&glyph) {
                self.flush();
            }
            self.current.push(glyph);
        }
        self.flush();
        self.lines
    }

    /// Whether `glyph` sits on the line being built: same baseline, and
    /// neither far to the right (another column) nor back to the left.
    fn continues(&self, glyph: &Glyph) -> bool {
        let Some(last) = self.current.last() else {
            return true;
        };
        let size = last.font_size.max(glyph.font_size).max(1.0);
        if (glyph.baseline - last.baseline).abs() > size * 0.5 {
            return false;
        }
        let gap = glyph.left - last.right;
        gap > -size && gap < size * 3.0
    }

    fn flush(&mut self) {
        let glyphs = std::mem::take(&mut self.current);
        if let Some(line) = build_line(glyphs) {
            self.lines.push(line);
        }
    }
}

fn build_line(glyphs: Vec<Glyph>) -> Option<PdfLine> {
    // Words are separated by what is visible: a space glyph counts only if
    // the next glyph does not sit back on top of it (troff kerns that way),
    // and a clear gap counts even without one.
    let mut text = String::new();
    let mut prev: Option<&Glyph> = None;
    let mut space_glyph = false;
    for g in &glyphs {
        if g.ch.is_whitespace() {
            space_glyph = true;
            continue;
        }
        if let Some(p) = prev {
            let gap = g.left - p.right;
            let size = g.font_size.min(p.font_size);
            if (space_glyph && gap > size * 0.1) || gap > size * 0.25 {
                text.push(' ');
            }
        }
        text.push(g.ch);
        prev = Some(g);
        space_glyph = false;
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let visible: Vec<&Glyph> = glyphs.iter().filter(|g| !g.ch.is_whitespace()).collect();
    let left = visible.iter().map(|g| g.left).fold(f32::MAX, f32::min);
    let right = visible.iter().map(|g| g.right).fold(f32::MIN, f32::max);
    let bottom = visible.iter().map(|g| g.bottom).fold(f32::MAX, f32::min);
    let top = visible.iter().map(|g| g.top).fold(f32::MIN, f32::max);
    let font_size = dominant_size(&visible);
    let bold_count = visible.iter().filter(|g| g.bold).count();
    Some(PdfLine {
        text,
        x: left,
        y: bottom,
        width: right - left,
        height: top - bottom,
        font_size,
        bold: bold_count * 2 > visible.len(),
    })
}

/// The font size most characters are set in, to a tenth of a point.
fn dominant_size(glyphs: &[&Glyph]) -> f32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for g in glyphs {
        *counts.entry((g.font_size * 10.0).round() as u32).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(size, n)| (n, size))
        .map(|(size, _)| size as f32 / 10.0)
        .unwrap_or(0.0)
}

/// Whether a character is set in a bold face. The font name is trusted
/// first: pdfium's weight for fonts without a descriptor (the standard
/// 14, as troff and old tools emit them) is a guess that is often wrong.
fn is_bold(c: &PdfPageTextChar<'_>) -> bool {
    let name = c.font_name().to_ascii_lowercase();
    if ["bold", "black", "heavy", "semibold"].iter().any(|w| name.contains(w)) {
        return true;
    }
    if ["roman", "regular", "light", "book", "medium", "italic", "oblique"]
        .iter()
        .any(|w| name.contains(w))
    {
        return false;
    }
    match c.font_weight() {
        Some(PdfFontWeight::Weight600)
        | Some(PdfFontWeight::Weight700Bold)
        | Some(PdfFontWeight::Weight800)
        | Some(PdfFontWeight::Weight900) => true,
        Some(PdfFontWeight::Custom(w)) => w >= 600,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page Letter PDF with Helvetica (F1) and Helvetica-Bold (F2)
    /// and the given content stream.
    fn pdf(content: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
            "<< /Title (Paper) /Author (Ada) >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn lines_come_with_geometry_sizes_and_weights() {
        let content = "\
BT /F1 9 Tf 72 760 Td (Journal of Things) Tj ET
BT /F2 16 Tf 72 700 Td (A Title) Tj ET
BT /F1 10 Tf 72 680 Td (Left column line one) Tj 0 -12 Td (left two) Tj ET
BT /F1 10 Tf 320 680 Td (Right column line one) Tj 0 -12 Td (right two) Tj ET
BT /F1 9 Tf 300 40 Td (1) Tj ET";
        let dir = tempdir();
        let path = dir.join("sample.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let text = PdfiumReader::new().read(&path).expect("pdfium reads the file");
        assert_eq!(text.title.as_deref(), Some("Paper"));
        assert_eq!(text.author.as_deref(), Some("Ada"));
        assert_eq!(text.pages.len(), 1);
        let page = &text.pages[0];
        assert_eq!((page.width, page.height), (612.0, 792.0));

        let texts: Vec<&str> = page.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Journal of Things",
                "A Title",
                "Left column line one",
                "left two",
                "Right column line one",
                "right two",
                "1",
            ]
        );
        let title = &page.lines[1];
        assert!((title.font_size - 16.0).abs() < 0.1, "{}", title.font_size);
        assert!(title.bold);
        assert!(!page.lines[0].bold);
        assert!((page.lines[2].font_size - 10.0).abs() < 0.1);
        assert!((page.lines[2].x - 72.0).abs() < 1.0);
        assert!((page.lines[4].x - 320.0).abs() < 1.0);
        assert!(page.lines[2].y > page.lines[3].y);
        assert!(page.lines[0].y > title.y && title.y > page.lines[2].y);
        assert!(page.lines[6].y < 50.0);
        assert!(page.lines[2].height > 8.0 && page.lines[2].height < 16.0);
        assert!(page.lines[2].width > 60.0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_files_give_none() {
        let dir = tempdir();
        let path = dir.join("not.pdf");
        std::fs::write(&path, b"hello").unwrap();
        assert!(PdfiumReader::new().read(&path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eddy-pdf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
