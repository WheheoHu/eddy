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
    if page.rules.len() < 2 {
        return Vec::new();
    }
    let (horizontal, vertical) = segments(&page.rules);
    let mut out: Vec<Table> = connected(&horizontal, &vertical)
        .into_iter()
        .filter_map(|(h, v)| {
            let h: Vec<Segment> = h.iter().map(|&i| horizontal[i]).collect();
            let v: Vec<Segment> = v.iter().map(|&i| vertical[i]).collect();
            grid_table(&h, &v, &page.lines)
        })
        .collect();
    let open = open_tables(&horizontal, &vertical, &page.lines, &out);
    out.extend(open);
    out
}

/// Tables ruled only across, as journals set them: two or more rules of
/// one width with nothing drawn down, around rows of text in columns.
/// Rules of that width elsewhere on the page (another table, a rule above
/// the footnotes) are apart wherever prose runs between two of them.
fn open_tables(h: &[Segment], v: &[Segment], lines: &[PdfLine], grids: &[Table]) -> Vec<Table> {
    // A rule through the foot of a line's text underlines it.
    let underline = |s: &Segment| {
        lines.iter().any(|l| {
            s.at >= l.y - 1.0
                && s.at <= l.y + l.height * 0.3
                && s.from < l.x + l.width
                && s.to > l.x
        })
    };
    let mut stacks: Vec<Vec<Segment>> = Vec::new();
    for s in h.iter().filter(|s| s.to - s.from >= 60.0 && !underline(s)) {
        match stacks
            .iter_mut()
            .find(|g| (g[0].from - s.from).abs() <= SNAP && (g[0].to - s.to).abs() <= SNAP)
        {
            Some(g) => g.push(*s),
            None => stacks.push(vec![*s]),
        }
    }
    let mut out = Vec::new();
    for mut stack in stacks {
        stack.sort_by(|a, b| b.at.total_cmp(&a.at));
        let (left, right) = (stack[0].from, stack[0].to);
        let between = |top: f32, bottom: f32| -> Vec<&PdfLine> {
            lines
                .iter()
                .filter(|l| {
                    let (cx, cy) = (l.x + l.width / 2.0, l.y + l.height / 2.0);
                    cx > left && cx < right && cy < top && cy > bottom && !l.text.trim().is_empty()
                })
                .collect()
        };
        let prose = |l: &PdfLine| wordy(l) && l.width >= (right - left) * 0.4;
        let mut start = 0;
        for k in 1..=stack.len() {
            let ends =
                k == stack.len() || between(stack[k - 1].at, stack[k].at).into_iter().any(prose);
            if !ends {
                continue;
            }
            let rules: Vec<f32> = stack[start..k].iter().map(|s| s.at).collect();
            start = k;
            if rules.len() < 2 {
                continue;
            }
            let (top, bottom) = (rules[0], rules[rules.len() - 1]);
            let crossed = v
                .iter()
                .any(|s| s.at > left && s.at < right && s.from < top && s.to > bottom);
            let taken = grids
                .iter()
                .any(|t| t.left < right && t.right > left && t.bottom < top && t.top > bottom);
            if crossed || taken {
                continue;
            }
            let (top, bottom) = beyond_rules(left, right, top, bottom, lines);
            if let Some(table) = open_table(left, right, top, bottom, &rules, &between(top, bottom))
            {
                out.push(table);
            }
        }
    }
    out
}

/// Lines as rows, top-down: lines on one baseline share a row.
fn rows_of<'l>(lines: impl IntoIterator<Item = &'l PdfLine>) -> Vec<Vec<&'l PdfLine>> {
    let mut sorted: Vec<&PdfLine> = lines.into_iter().collect();
    sorted.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut rows: Vec<Vec<&PdfLine>> = Vec::new();
    for line in sorted {
        match rows.last_mut() {
            Some(row) if (row[0].y - line.y).abs() < row[0].font_size.max(1.0) * 0.5 => {
                row.push(line)
            }
            _ => rows.push(vec![line]),
        }
    }
    rows
}

/// How far a table reaches past its outermost rules (some are drawn so
/// that they do not read as rules): rows right above or below at the
/// table's pitch, within its width, that are not running text.
fn beyond_rules(left: f32, right: f32, top: f32, bottom: f32, lines: &[PdfLine]) -> (f32, f32) {
    let rows = rows_of(lines.iter().filter(|l| {
        let cx = l.x + l.width / 2.0;
        cx > left && cx < right && !l.text.trim().is_empty()
    }));
    let middle = |row: &[&PdfLine]| row[0].y + row[0].height / 2.0;
    let inside: Vec<usize> = (0..rows.len())
        .filter(|&r| middle(&rows[r]) < top && middle(&rows[r]) > bottom)
        .collect();
    let (Some(&first), Some(&last)) = (inside.first(), inside.last()) else {
        return (top, bottom);
    };
    let mut gaps: Vec<f32> = (first..last)
        .map(|r| rows[r][0].y - rows[r + 1][0].y)
        .collect();
    gaps.sort_by(f32::total_cmp);
    let pitch = gaps
        .get(gaps.len() / 2)
        .copied()
        .unwrap_or(rows[first][0].height * 1.3);
    let fits = |row: &[&PdfLine]| {
        row.iter().all(|l| {
            !wordy(l)
                && !is_table_caption(&l.text)
                && l.x >= left - SNAP
                && l.x + l.width <= right + SNAP
        })
    };
    let cells = |row: &[&PdfLine]| row.iter().map(|l| line_runs(l).len()).sum::<usize>();
    let close = |upper: usize| rows[upper][0].y - rows[upper + 1][0].y <= pitch * 1.8;
    let (mut start, mut end) = (first, last);
    // Upwards, column headings: a row of one cell (a caption, a running
    // head) only under a row of several.
    while start > 0 && close(start - 1) && fits(&rows[start - 1]) {
        let k = start - 1;
        let several = cells(&rows[k]) >= 2;
        let under_several = k > 0 && close(k - 1) && fits(&rows[k - 1]) && cells(&rows[k - 1]) >= 2;
        if several {
            start = k;
        } else if under_several {
            start = k - 1;
        } else {
            break;
        }
    }
    while end + 1 < rows.len()
        && rows[end][0].y - rows[end + 1][0].y <= pitch * 1.8
        && fits(&rows[end + 1])
    {
        end += 1;
    }
    let row_top = rows[start]
        .iter()
        .map(|l| l.y + l.height)
        .fold(f32::MIN, f32::max);
    let row_bottom = rows[end].iter().map(|l| l.y).fold(f32::MAX, f32::min);
    (top.max(row_top + 0.5), bottom.min(row_bottom - 0.5))
}

/// The table between `top` and `bottom` with `rules` (descending) in it,
/// from its lines: rows by baseline, columns where no run on any body
/// row reaches across. Rows above the first rule, and a row set off by
/// rules above and below, head what follows.
fn open_table(
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    rules: &[f32],
    lines: &[&PdfLine],
) -> Option<Table> {
    let rows = rows_of(lines.iter().copied());
    if rows.len() < 2 || rows.iter().filter(|r| r.iter().any(|l| wordy(l))).count() * 3 > rows.len()
    {
        return None;
    }
    let size = lines.iter().map(|l| l.font_size).fold(0.0, f32::max);
    let row_runs: Vec<Vec<crate::PdfRun>> = rows
        .iter()
        .map(|row| {
            let mut runs: Vec<crate::PdfRun> =
                row.iter().flat_map(|l| line_runs(l).into_owned()).collect();
            runs.sort_by(|a, b| a.x.total_cmp(&b.x));
            // Runs that touch differ only in style: one value.
            let mut merged: Vec<crate::PdfRun> = Vec::with_capacity(runs.len());
            for run in runs {
                match merged.last_mut() {
                    Some(prev) if run.x - (prev.x + prev.width) < size * 0.3 => {
                        if run.space_before {
                            prev.text.push(' ');
                        }
                        prev.text.push_str(&run.text);
                        prev.width = run.x + run.width - prev.x;
                    }
                    _ => merged.push(run),
                }
            }
            merged
        })
        .collect();
    // Columns: what the fullest rows cover across, merged over small gaps.
    // A heading over several columns, or a value set between two, does not
    // join them.
    let most = row_runs.iter().map(Vec::len).max().unwrap_or(0);
    let full: Vec<&Vec<crate::PdfRun>> = row_runs.iter().filter(|r| r.len() == most).collect();
    let full: Vec<&Vec<crate::PdfRun>> = if full.len() >= 2 {
        full
    } else {
        row_runs.iter().filter(|r| r.len() + 1 >= most).collect()
    };
    let mut spans: Vec<(f32, f32)> = full
        .iter()
        .copied()
        .flatten()
        .map(|r| (r.x, r.x + r.width))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut columns: Vec<(f32, f32)> = Vec::new();
    for (a, b) in spans {
        match columns.last_mut() {
            Some(c) if a <= c.1 + size * 0.6 => c.1 = c.1.max(b),
            _ => columns.push((a, b)),
        }
    }
    if columns.len() < 2 {
        return None;
    }
    // Each column reaches halfway to its neighbours; a run spans the
    // columns its ends fall in (a heading over two columns sits across
    // the gap between them).
    let region = |x: f32| -> usize {
        (0..columns.len() - 1)
            .find(|&c| x < (columns[c].1 + columns[c + 1].0) / 2.0)
            .unwrap_or(columns.len() - 1)
    };
    let span_of = |run: &crate::PdfRun| -> (usize, usize) {
        let slack = (size * 0.3).min(run.width / 2.0);
        let first = region(run.x + slack);
        (first, region(run.x + run.width - slack).max(first))
    };
    let mut table_rows: Vec<Vec<Cell>> = Vec::new();
    for (row, runs) in rows.iter().zip(&row_runs) {
        let mut cells: Vec<(usize, usize, Vec<crate::PdfRun>)> = Vec::new();
        for run in runs {
            let (first, last) = span_of(run);
            match cells.last_mut() {
                Some((at, span, parts)) if *at + *span > first => {
                    *span = (*span).max(last + 1 - *at);
                    parts.push(run.clone());
                }
                _ => cells.push((first, last + 1 - first, vec![run.clone()])),
            }
        }
        let mut out_row: Vec<Cell> = Vec::new();
        let mut next = 0;
        for (at, span, parts) in cells {
            while next < at {
                out_row.push(empty_cell());
                next += 1;
            }
            out_row.push(Cell {
                lines: vec![sub_line(row[0], &parts)],
                colspan: span,
                rowspan: 1,
                header: false,
            });
            next = at + span;
        }
        while next < columns.len() {
            out_row.push(empty_cell());
            next += 1;
        }
        table_rows.push(out_row);
    }
    let filled = table_rows
        .iter()
        .filter(|r| r.iter().filter(|c| !c.lines.is_empty()).count() >= 2)
        .count();
    if filled * 2 < table_rows.len() {
        return None;
    }
    // Header rows: a rule right above and right below.
    let ruled_between = |upper: f32, lower: f32| rules.iter().any(|&y| y < upper && y > lower);
    for r in 0..rows.len() {
        let top = rows[r]
            .iter()
            .map(|l| l.y + l.height)
            .fold(f32::MIN, f32::max);
        let bottom = rows[r].iter().map(|l| l.y).fold(f32::MAX, f32::min);
        let above = if r == 0 {
            true
        } else {
            ruled_between(rows[r - 1][0].y, top)
        };
        let below =
            r + 1 < rows.len() && ruled_between(bottom, rows[r + 1][0].y + rows[r + 1][0].height);
        if (above && below) || bottom > rules[0] {
            for cell in table_rows[r].iter_mut() {
                cell.header = true;
            }
        }
    }
    Some(Table {
        left,
        bottom,
        right,
        top,
        rows: table_rows,
    })
}

/// "Table 3: …", "Tab. 2", "表 1".
fn is_table_caption(text: &str) -> bool {
    let t = text.trim_start();
    ["Table", "TABLE", "Tab.", "表"].iter().any(|p| {
        t.strip_prefix(p).is_some_and(|rest| {
            rest.trim_start()
                .starts_with(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase())
        })
    })
}

/// A stretch of running text: one run of eight words or more.
fn wordy(line: &PdfLine) -> bool {
    line_runs(line)
        .iter()
        .any(|r| r.text.split_whitespace().count() >= 8)
}

fn empty_cell() -> Cell {
    Cell {
        lines: Vec::new(),
        colspan: 1,
        rowspan: 1,
        header: false,
    }
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
                mono: false,
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
    fn rules_only_across_make_a_table_headed_by_the_row_they_set_off() {
        let mut lines = vec![cell_line("Table 1: Results", 100.0, 720.0)];
        for (y, row) in [
            (690.0, ["Model", "BLEU", "Cost"]),
            (670.0, ["alpha", "1.0", "2.0"]),
            (658.0, ["beta", "3.0", "4.0"]),
            (646.0, ["gamma", "5.0", "6.0"]),
        ] {
            for (x, text) in [100.0, 220.0, 320.0].into_iter().zip(row) {
                lines.push(cell_line(text, x, y));
            }
        }
        lines.push(cell_line(
            "Below the table runs a sentence of plain words",
            100.0,
            600.0,
        ));
        let page = PdfPage {
            width: 612.0,
            height: 792.0,
            lines,
            rules: vec![
                rule(90.0, 705.0, 300.0, 0.5),
                rule(90.0, 686.0, 300.0, 0.5),
                rule(90.0, 640.0, 300.0, 0.5),
            ],
            ..Default::default()
        };
        let tables = tables(&page);
        assert_eq!(tables.len(), 1, "{tables:?}");
        let rows: Vec<Vec<String>> = tables[0]
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| c.lines.iter().map(|l| l.text.clone()).collect())
                    .collect()
            })
            .collect();
        assert_eq!(
            rows,
            [
                ["Model", "BLEU", "Cost"],
                ["alpha", "1.0", "2.0"],
                ["beta", "3.0", "4.0"],
                ["gamma", "5.0", "6.0"],
            ]
        );
        assert!(tables[0].rows[0].iter().all(|c| c.header));
        assert!(tables[0].rows[1].iter().all(|c| !c.header));
        assert!(!tables[0].holds(&page.lines[0]), "the caption stays out");
    }

    #[test]
    fn lone_rules_and_open_boxes_are_not_tables() {
        let mut page = page();
        // Without its right border the grid is not closed.
        page.rules
            .retain(|r| !(r.height > r.width && (r.x - 399.75).abs() < 0.1));
        assert!(tables(&page).is_empty());
        // Rules through the foot of each line underline it.
        let underlines = PdfPage {
            rules: page
                .lines
                .iter()
                .map(|l| rule(l.x - 50.0, l.y + 1.0, l.width + 100.0, 0.5))
                .collect(),
            ..page
        };
        assert!(tables(&underlines).is_empty());
    }
}
