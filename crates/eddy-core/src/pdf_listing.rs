//! Code in a PDF: listings framed by a box and laid out as code (indents
//! stepping in and out, a ragged right edge), and lines set in a
//! typewriter face. Either reads as preformatted text with its indents
//! restored; prose in a box (a callout) stays ordinary text.

use crate::import::{PdfBox, PdfImage, PdfLine, PdfPage};
use crate::pdf_layout::line_runs;

/// A boxed listing on a page and its text.
#[derive(Debug)]
pub(crate) struct Listing {
    pub(crate) left: f32,
    pub(crate) bottom: f32,
    pub(crate) right: f32,
    pub(crate) top: f32,
    pub(crate) text: String,
}

impl Listing {
    /// Whether a line of the page sits in the listing.
    pub(crate) fn holds(&self, line: &PdfLine) -> bool {
        let (cx, cy) = center(line);
        cx > self.left && cx < self.right && cy > self.bottom && cy < self.top
    }

    pub(crate) fn center_inside(&self, image: &PdfImage) -> bool {
        let cx = (self.left + self.right) / 2.0;
        let cy = (self.bottom + self.top) / 2.0;
        cx > image.x && cx < image.x + image.width && cy > image.y && cy < image.y + image.height
    }
}

fn center(line: &PdfLine) -> (f32, f32) {
    (line.x + line.width / 2.0, line.y + line.height / 2.0)
}

fn contains(b: &PdfBox, line: &PdfLine) -> bool {
    let (cx, cy) = center(line);
    cx > b.x && cx < b.x + b.width && cy > b.y && cy < b.y + b.height && line.width <= b.width + 2.0
}

/// Boxes on the page whose lines read as code. Each line belongs to the
/// smallest box around it, so a panel around two listings is not one.
/// Lines `skip` says are taken (by a table) are left out.
pub(crate) fn listings(page: &PdfPage, skip: impl Fn(&PdfLine) -> bool) -> Vec<Listing> {
    let mut held: Vec<Vec<&PdfLine>> = vec![Vec::new(); page.boxes.len()];
    for line in page
        .lines
        .iter()
        .filter(|l| !l.text.trim().is_empty() && !skip(l))
    {
        let smallest = page
            .boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| contains(b, line))
            .min_by(|(_, a), (_, b)| (a.width * a.height).total_cmp(&(b.width * b.height)));
        if let Some((i, _)) = smallest {
            held[i].push(line);
        }
    }
    let mut out = Vec::new();
    for (b, mut lines) in page.boxes.iter().zip(held) {
        if !is_code(&lines) {
            continue;
        }
        lines.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
        // A rectangle drawn twice (filled, then outlined) is one listing.
        let same = |l: &Listing| (l.left - b.x).abs() < 1.0 && (l.top - b.y - b.height).abs() < 1.0;
        if out.iter().any(same) {
            continue;
        }
        out.push(Listing {
            left: b.x,
            bottom: b.y,
            right: b.x + b.width,
            top: b.y + b.height,
            text: code_text(&lines),
        });
    }
    out
}

/// Whether most of a line is set in a typewriter face.
pub(crate) fn is_mono(line: &PdfLine) -> bool {
    let count = |mono: bool| -> usize {
        line_runs(line)
            .iter()
            .filter(|r| r.mono == mono)
            .map(|r| r.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum()
    };
    let mono = count(true);
    mono > 0 && mono >= count(false) * 2
}

/// Whether the lines in a box are code: all typewriter text, or set like
/// code in any face: at least three lines at an even pitch over three or
/// more indent levels, starting at the outermost, with a ragged right
/// edge, few lines as wordy as prose, and no bullets (a callout's list).
/// Labels scattered over a drawing are not evenly pitched.
fn is_code(lines: &[&PdfLine]) -> bool {
    if lines.len() >= 2 && lines.iter().all(|l| is_mono(l)) {
        return true;
    }
    if lines.len() < 3 {
        return false;
    }
    let size = median(lines.iter().map(|l| l.font_size).collect());
    let mut sorted = lines.to_vec();
    sorted.sort_by(|a, b| b.y.total_cmp(&a.y));
    let mut rows: Vec<f32> = Vec::new();
    for l in &sorted {
        if rows.last().is_none_or(|&y| y - l.y >= size * 0.5) {
            rows.push(l.y);
        }
    }
    let gaps: Vec<f32> = rows.windows(2).map(|w| w[0] - w[1]).collect();
    let pitch = median(gaps.clone());
    let even = pitch >= size * 0.9 && pitch <= size * 2.0 && gaps.iter().all(|&g| g <= pitch * 3.0);
    let levels = indent_levels(lines, size);
    let starts_outermost = levels
        .first()
        .is_some_and(|&x| sorted[0].x <= x + size * 0.35);
    let right = lines.iter().map(|l| l.x + l.width).fold(f32::MIN, f32::max);
    let full = lines
        .iter()
        .filter(|l| l.x + l.width >= right - size * 1.5)
        .count();
    let bullets = lines
        .iter()
        .filter(|l| {
            l.text
                .trim_start()
                .starts_with(['•', '●', '○', '◦', '▪', '■', '◆', '‣', '-', '–'])
        })
        .count();
    let wordy = lines
        .iter()
        .filter(|l| l.text.split_whitespace().count() >= 10)
        .count();
    rows.len() >= 3
        && wordy * 4 <= lines.len()
        && even
        && starts_outermost
        && levels.len() >= 3
        && full * 2 <= lines.len()
        && bullets * 3 < lines.len()
}

fn median(mut values: Vec<f32>) -> f32 {
    values.sort_by(f32::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

/// Distinct left edges, ascending, merged within a third of the type
/// size.
fn indent_levels(lines: &[&PdfLine], size: f32) -> Vec<f32> {
    let mut xs: Vec<f32> = lines.iter().map(|l| l.x).collect();
    xs.sort_by(f32::total_cmp);
    let mut levels: Vec<f32> = Vec::new();
    for x in xs {
        match levels.last() {
            Some(&last) if x - last <= size * 0.35 => {}
            _ => levels.push(x),
        }
    }
    levels
}

/// The lines (in reading order) as preformatted text: pieces on one row
/// join, indents become spaces, and a blank line stands for a wide gap.
/// Typewriter text keeps its columns; proportional text indents two
/// spaces a level.
pub(crate) fn code_text(lines: &[&PdfLine]) -> String {
    let mut rows: Vec<Vec<&PdfLine>> = Vec::new();
    for &line in lines {
        match rows.last_mut() {
            Some(row)
                if (row[0].y - line.y).abs() < row[0].font_size * 0.5 && line.x > row[0].x =>
            {
                row.push(line);
            }
            _ => rows.push(vec![line]),
        }
    }
    let size = median(lines.iter().map(|l| l.font_size).collect());
    let mono = lines.iter().filter(|l| is_mono(l)).count() * 2 > lines.len();
    let cell = median(
        lines
            .iter()
            .flat_map(|l| line_runs(l).into_owned())
            .filter(|r| r.mono)
            .filter_map(|r| {
                let n = r.text.chars().count();
                (n > 0).then(|| r.width / n as f32)
            })
            .collect(),
    );
    let cell = if cell > 0.0 { cell } else { size * 0.6 };
    let levels = indent_levels(lines, size);
    let left = levels.first().copied().unwrap_or(0.0);
    let indent = |x: f32| -> usize {
        if mono {
            ((x - left) / cell).round().max(0.0) as usize
        } else {
            2 * levels.iter().filter(|&&l| l < x - size * 0.35).count()
        }
    };

    let mut out = String::new();
    let mut prev: Option<&PdfLine> = None;
    for row in &rows {
        let first = row[0];
        if let Some(p) = prev {
            out.push('\n');
            let line_height = p.height.max(first.height).max(size);
            if first.y < p.y && p.y - first.y > line_height * 1.8 {
                out.push('\n');
            }
        }
        let mut text = " ".repeat(indent(first.x));
        let mut right_edge: Option<f32> = None;
        for (i, piece) in row.iter().enumerate() {
            if !mono {
                if i > 0 {
                    text.push(' ');
                }
                text.push_str(&piece.text);
                continue;
            }
            // A gap between stretches is as many spaces as character
            // cells fit in it.
            for (k, run) in line_runs(piece).iter().enumerate() {
                if let Some(end) = right_edge {
                    let least = usize::from(k == 0 || run.space_before);
                    let cells = ((run.x - end) / cell).round().max(0.0) as usize;
                    text.extend(std::iter::repeat_n(' ', cells.max(least)));
                }
                text.push_str(&run.text);
                right_edge = Some(run.x + run.width);
            }
        }
        out.push_str(text.trim_end());
        prev = Some(first);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::PdfRun;

    fn line(text: &str, x: f32, y: f32, mono: bool) -> PdfLine {
        let width = text.chars().count() as f32 * 3.6;
        PdfLine {
            text: text.into(),
            x,
            y,
            width,
            height: 6.0,
            font_size: 6.0,
            bold: false,
            runs: vec![PdfRun {
                text: text.into(),
                x,
                width,
                mono,
                ..PdfRun::default()
            }],
        }
    }

    fn boxed(lines: Vec<PdfLine>) -> PdfPage {
        PdfPage {
            width: 600.0,
            height: 800.0,
            lines,
            boxes: vec![PdfBox {
                x: 300.0,
                y: 600.0,
                width: 120.0,
                height: 120.0,
            }],
            ..PdfPage::default()
        }
    }

    #[test]
    fn stepped_lines_in_a_box_are_a_listing_with_their_indents() {
        let page = boxed(vec![
            line("def render(px):", 304.0, 700.0, false),
            line("I ← 0", 307.0, 692.0, false),
            line("for i = 0..N:", 307.0, 684.0, false),
            line("x ← trace(px)", 310.0, 676.0, false),
            line("I ← I + x", 310.0, 668.0, false),
            line("return I", 307.0, 660.0, false),
        ]);
        let found = listings(&page, |_| false);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].text,
            "def render(px):\n  I ← 0\n  for i = 0..N:\n    x ← trace(px)\n    I ← I + x\n  return I"
        );
        assert!(found[0].holds(&page.lines[3]));
    }

    #[test]
    fn prose_in_a_box_is_not_a_listing() {
        // A callout: wrapped lines reaching the right edge.
        let page = boxed(vec![
            line("Note: keep the unit away from", 304.0, 700.0, false),
            line("water and direct sunlight to", 304.0, 692.0, false),
            line("avoid damage to the display.", 304.0, 684.0, false),
        ]);
        assert!(listings(&page, |_| false).is_empty());
    }

    #[test]
    fn typewriter_lines_keep_their_columns() {
        let a = line("let x = 1;", 304.0, 700.0, true);
        let b = line("x += 2;", 304.0 + 4.0 * 3.6, 692.0, true);
        let text = code_text(&[&a, &b]);
        assert_eq!(text, "let x = 1;\n    x += 2;");
    }
}
