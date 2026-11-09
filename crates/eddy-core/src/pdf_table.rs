//! Tables drawn with rules in a PDF: horizontal and vertical lines that
//! meet in a closed grid. Cells are the grid's boxes, merged where a
//! border between them is missing; the text in each box is its content.
//! Tables without borders read as ordinary text.

use crate::import::{PdfImage, PdfLine, PdfPage, PdfRule};
use crate::pdf_layout::{line_runs, sub_line};

/// How far apart two rules can be and still be one line, in points.
const SNAP: f32 = 2.0;

/// A table on a page: its box and rows of cells.
#[derive(Debug)]
pub(crate) struct Table {
    pub(crate) left: f32,
    pub(crate) bottom: f32,
    pub(crate) right: f32,
    pub(crate) top: f32,
    pub(crate) rows: Vec<Vec<Cell>>,
}

#[derive(Debug)]
pub(crate) struct Cell {
    /// The cell's text, top-down.
    pub(crate) lines: Vec<PdfLine>,
    pub(crate) colspan: usize,
    pub(crate) rowspan: usize,
    pub(crate) header: bool,
}

impl Table {
    /// Whether a line of the page sits in the table.
    pub(crate) fn holds(&self, line: &PdfLine) -> bool {
        let cx = line.x + line.width / 2.0;
        let cy = line.y + line.height / 2.0;
        cx > self.left && cx < self.right && cy > self.bottom && cy < self.top
    }

    pub(crate) fn center_inside(&self, image: &PdfImage) -> bool {
        let cx = (self.left + self.right) / 2.0;
        let cy = (self.bottom + self.top) / 2.0;
        cx > image.x && cx < image.x + image.width && cy > image.y && cy < image.y + image.height
    }
}

/// A rule as a segment: `at` across, from `from` to `to` along it.
#[derive(Clone, Copy, Debug)]
struct Segment {
    at: f32,
    from: f32,
    to: f32,
}

/// Rules that continue one another (a border drawn cell by cell) joined
/// into one segment.
fn merge(mut segments: Vec<Segment>) -> Vec<Segment> {
    segments.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    let mut out: Vec<Segment> = Vec::new();
    for s in segments {
        let joined = out
            .iter_mut()
            .rev()
            .take_while(|o| s.at - o.at <= SNAP)
            .find(|o| {
                (o.at - s.at).abs() <= SNAP && s.from <= o.to + SNAP && s.to >= o.from - SNAP
            });
        match joined {
            Some(o) => {
                o.from = o.from.min(s.from);
                o.to = o.to.max(s.to);
            }
            None => out.push(s),
        }
    }
    out
}

/// Values within `SNAP` of each other as one, ascending.
fn distinct(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    let mut out: Vec<f32> = Vec::new();
    for v in values {
        if out.last().is_none_or(|&last| v - last > SNAP) {
            out.push(v);
        }
    }
    out
}

fn segments(rules: &[PdfRule]) -> (Vec<Segment>, Vec<Segment>) {
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    for r in rules {
        if r.height <= r.width {
            horizontal.push(Segment {
                at: r.y + r.height / 2.0,
                from: r.x,
                to: r.x + r.width,
            });
        } else {
            vertical.push(Segment {
                at: r.x + r.width / 2.0,
                from: r.y,
                to: r.y + r.height,
            });
        }
    }
    (merge(horizontal), merge(vertical))
}

/// Groups of rules that cross or touch one another, as indices into the
/// horizontal and vertical segments.
fn connected(horizontal: &[Segment], vertical: &[Segment]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let n = horizontal.len();
    let mut parent: Vec<usize> = (0..n + vertical.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, h) in horizontal.iter().enumerate() {
        for (j, v) in vertical.iter().enumerate() {
            let meet = v.at >= h.from - SNAP
                && v.at <= h.to + SNAP
                && h.at >= v.from - SNAP
                && h.at <= v.to + SNAP;
            if meet {
                let (a, b) = (root(&mut parent, i), root(&mut parent, n + j));
                parent[b] = a;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<usize>, Vec<usize>)> = Vec::new();
    for i in 0..parent.len() {
        let r = root(&mut parent, i);
        let at = match groups.iter().position(|g| g.0 == r) {
            Some(at) => at,
            None => {
                groups.push((r, Vec::new(), Vec::new()));
                groups.len() - 1
            }
        };
        if i < n {
            groups[at].1.push(i);
        } else {
            groups[at].2.push(i - n);
        }
    }
    groups.into_iter().map(|(_, h, v)| (h, v)).collect()
}

/// The tables on a page.
pub(crate) fn tables(page: &PdfPage) -> Vec<Table> {
    if page.rules.len() < 4 {
        return Vec::new();
    }
    let (horizontal, vertical) = segments(&page.rules);
    connected(&horizontal, &vertical)
        .into_iter()
        .filter_map(|(h, v)| {
            let h: Vec<Segment> = h.iter().map(|&i| horizontal[i]).collect();
            let v: Vec<Segment> = v.iter().map(|&i| vertical[i]).collect();
            grid_table(&h, &v, &page.lines)
        })
        .collect()
}

/// A table from one connected set of rules, when they make a closed grid
/// of at least two rows and two columns with text in it.
fn grid_table(h: &[Segment], v: &[Segment], lines: &[PdfLine]) -> Option<Table> {
    let xs = distinct(v.iter().map(|s| s.at).collect());
    let mut ys = distinct(h.iter().map(|s| s.at).collect());
    ys.reverse();
    if xs.len() < 3 || ys.len() < 3 {
        return None;
    }
    let (left, right) = (xs[0], xs[xs.len() - 1]);
    let (top, bottom) = (ys[0], ys[ys.len() - 1]);
    // The frame is closed: rules along all four sides.
    let spans = |segs: &[Segment], at: f32, from: f32, to: f32| {
        segs.iter()
            .any(|s| (s.at - at).abs() <= SNAP && s.from <= from + SNAP && s.to >= to - SNAP)
    };
    let width = right - left;
    let height = top - bottom;
    let framed = spans(h, top, left + width * 0.05, right - width * 0.05)
        && spans(h, bottom, left + width * 0.05, right - width * 0.05)
        && spans(v, left, bottom + height * 0.05, top - height * 0.05)
        && spans(v, right, bottom + height * 0.05, top - height * 0.05);
    if !framed {
        return None;
    }
    let (rows, cols) = (ys.len() - 1, xs.len() - 1);
    // A border is there where some rule covers the middle of the edge.
    let has_v = |k: usize, r: usize| {
        let mid = (ys[r] + ys[r + 1]) / 2.0;
        v.iter()
            .any(|s| (s.at - xs[k]).abs() <= SNAP && s.from <= mid && s.to >= mid)
    };
    let has_h = |k: usize, c: usize| {
        let mid = (xs[c] + xs[c + 1]) / 2.0;
        h.iter()
            .any(|s| (s.at - ys[k]).abs() <= SNAP && s.from <= mid && s.to >= mid)
    };
    // Cells: each grid box belongs to the cell anchored at its top-left.
    let mut owner: Vec<Vec<Option<(usize, usize)>>> = vec![vec![None; cols]; rows];
    let mut anchors: Vec<(usize, usize, usize, usize)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if owner[r][c].is_some() {
                continue;
            }
            let mut colspan = 1;
            while c + colspan < cols && !has_v(c + colspan, r) && owner[r][c + colspan].is_none() {
                colspan += 1;
            }
            let mut rowspan = 1;
            while r + rowspan < rows
                && (c..c + colspan)
                    .all(|cc| !has_h(r + rowspan, cc) && owner[r + rowspan][cc].is_none())
            {
                rowspan += 1;
            }
            for row in owner.iter_mut().skip(r).take(rowspan) {
                for slot in row.iter_mut().skip(c).take(colspan) {
                    *slot = Some((r, c));
                }
            }
            anchors.push((r, c, rowspan, colspan));
        }
    }
    // Text: each run goes to the cell under its middle.
    let find = |values: &[f32], v: f32, descending: bool| -> Option<usize> {
        (0..values.len() - 1).find(|&i| {
            let (a, b) = (values[i], values[i + 1]);
            if descending {
                v <= a && v > b
            } else {
                v >= a && v < b
            }
        })
    };
    let mut content: Vec<Vec<PdfLine>> = vec![Vec::new(); anchors.len()];
    for line in lines {
        let cy = line.y + line.height / 2.0;
        let Some(r) = find(&ys, cy, true) else {
            continue;
        };
        let runs = line_runs(line);
        let mut i = 0;
        while i < runs.len() {
            let cell_of = |run: &crate::PdfRun| {
                find(&xs, run.x + run.width / 2.0, false).and_then(|c| owner[r][c])
            };
            let cell = cell_of(&runs[i]);
            let mut j = i + 1;
            while j < runs.len() && cell_of(&runs[j]) == cell {
                j += 1;
            }
            if let Some(anchor) = cell
                && let Some(at) = anchors.iter().position(|a| (a.0, a.1) == anchor)
            {
                content[at].push(sub_line(line, &runs[i..j]));
            }
            i = j;
        }
    }
    let filled = content.iter().filter(|c| !c.is_empty()).count();
    if anchors.len() < 4 || filled * 2 < anchors.len() {
        return None;
    }
    let mut table_rows: Vec<Vec<Cell>> = (0..rows).map(|_| Vec::new()).collect();
    for ((r, _, rowspan, colspan), mut lines) in anchors.into_iter().zip(content) {
        lines.sort_by(|a, b| {
            (b.y + b.height)
                .total_cmp(&(a.y + a.height))
                .then(a.x.total_cmp(&b.x))
        });
        table_rows[r].push(Cell {
            lines,
            colspan,
            rowspan,
            header: false,
        });
    }
    // A first row set wholly in bold heads the columns.
    if let Some(first) = table_rows.first_mut()
        && first.len() > 1
        && first
            .iter()
            .all(|c| !c.lines.is_empty() && c.lines.iter().all(|l| l.bold))
    {
        for cell in first.iter_mut() {
            cell.header = true;
        }
    }
    table_rows.retain(|r| !r.is_empty());
    Some(Table {
        left,
        bottom,
        right,
        top,
        rows: table_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::PdfRun;

    fn rule(x: f32, y: f32, width: f32, height: f32) -> PdfRule {
        PdfRule {
            x,
            y,
            width,
            height,
        }
    }

    fn cell_line(text: &str, x: f32, y: f32) -> PdfLine {
        let width = text.chars().count() as f32 * 10.0;
        PdfLine {
            text: text.into(),
            x,
            y,
            width,
            height: 10.0,
            font_size: 10.0,
            bold: false,
            runs: vec![PdfRun {
                text: text.into(),
                x,
                width,
                bold: false,
                underline: false,
                space_before: false,
            }],
        }
    }

    /// A 3-column, 3-row grid from x=100 to 400 and y=500 down to 440,
    /// drawn cell by cell, with the first row's inner borders missing
    /// (one cell across) and the left column's middle border missing (a
    /// cell two rows high).
    fn page() -> PdfPage {
        let mut rules = Vec::new();
        let xs = [100.0, 200.0, 300.0, 400.0];
        let ys = [500.0, 480.0, 460.0, 440.0];
        for (k, &y) in ys.iter().enumerate() {
            for (c, &x) in xs.iter().take(3).enumerate() {
                if k == 2 && c == 0 {
                    continue;
                }
                rules.push(rule(x, y - 0.25, 100.0, 0.5));
            }
        }
        for (k, &x) in xs.iter().enumerate() {
            for r in 0..3 {
                if r == 0 && (k == 1 || k == 2) {
                    continue;
                }
                rules.push(rule(x - 0.25, ys[r + 1], 0.5, 20.0));
            }
        }
        let mut lines = vec![
            cell_line("Title", 220.0, 485.0),
            cell_line("Stage", 110.0, 455.0),
            cell_line("T", 210.0, 465.0),
            cell_line("N", 310.0, 465.0),
            cell_line("1", 210.0, 445.0),
            cell_line("0", 310.0, 445.0),
            cell_line("Below the table", 100.0, 400.0),
        ];
        // "T" and "N" read as one line with two runs.
        let n = lines.remove(3);
        let t = &mut lines[2];
        t.text = "T N".into();
        t.width = n.x + n.width - t.x;
        t.runs.push(PdfRun {
            space_before: true,
            ..n.runs[0].clone()
        });
        PdfPage {
            width: 612.0,
            height: 792.0,
            lines,
            rules,
            ..Default::default()
        }
    }

    #[test]
    fn a_ruled_grid_is_a_table_with_merged_cells() {
        let page = page();
        let tables = tables(&page);
        assert_eq!(tables.len(), 1, "{tables:?}");
        let t = &tables[0];
        assert!((t.left - 100.0).abs() < 1.0 && (t.top - 500.0).abs() < 1.0);
        let shape: Vec<Vec<(String, usize, usize)>> = t
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| {
                        let text: Vec<&str> = c.lines.iter().map(|l| l.text.as_str()).collect();
                        (text.join("/"), c.colspan, c.rowspan)
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            shape,
            [
                vec![("Title".to_string(), 3, 1)],
                vec![
                    ("Stage".to_string(), 1, 2),
                    ("T".to_string(), 1, 1),
                    ("N".to_string(), 1, 1)
                ],
                vec![("1".to_string(), 1, 1), ("0".to_string(), 1, 1)],
            ]
        );
        assert!(t.holds(&page.lines[0]));
        assert!(!t.holds(page.lines.last().unwrap()));
    }

    #[test]
    fn lone_rules_and_open_boxes_are_not_tables() {
        let mut page = page();
        // Without its right border the grid is not closed.
        page.rules
            .retain(|r| !(r.height > r.width && (r.x - 399.75).abs() < 0.1));
        assert!(tables(&page).is_empty());
        let underlines = PdfPage {
            rules: (0..6)
                .map(|i| rule(100.0, 500.0 - 14.0 * i as f32, 300.0, 0.5))
                .collect(),
            ..page
        };
        assert!(tables(&underlines).is_empty());
    }
}
