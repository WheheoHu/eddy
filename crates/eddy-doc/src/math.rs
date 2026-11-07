use eddy_model::{BlockKind, Run, style};
use scraper::ElementRef;

/// LaTeX source of a rendered formula: KaTeX and MathJax keep it in a MathML
/// `<annotation encoding="application/x-tex">`; plain MathML may carry
/// `alttext`. Falls back to the visible text.
pub fn tex_in(el: ElementRef) -> Option<String> {
    raw_tex_in(el).map(|t| normalize_tex(&t))
}

/// TeX source of a Wikipedia (Parsoid) formula, kept in the `data-mw`
/// attribute as `{"body": {"extsrc": "…"}}`.
pub fn tex_from_data_mw(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let tex = value.get("body")?.get("extsrc")?.as_str()?;
    Some(normalize_tex(tex))
}

/// Unwraps `{\displaystyle …}` (Wikipedia) and trims.
pub fn normalize_tex(tex: &str) -> String {
    let t = tex.trim();
    for wrapper in ["{\\displaystyle", "{\\textstyle"] {
        if let Some(inner) = t.strip_prefix(wrapper).and_then(|r| r.strip_suffix('}')) {
            return inner.trim().to_string();
        }
    }
    t.to_string()
}

fn raw_tex_in(el: ElementRef) -> Option<String> {
    let annotation = el.descendants().filter_map(ElementRef::wrap).find(|e| {
        e.value().name() == "annotation"
            && e.value().attr("encoding").is_some_and(|enc| {
                enc.eq_ignore_ascii_case("application/x-tex") || enc.eq_ignore_ascii_case("tex")
            })
    });
    if let Some(a) = annotation {
        return Some(a.text().collect::<String>().trim().to_string());
    }
    let math = if el.value().name() == "math" {
        Some(el)
    } else {
        el.descendants()
            .filter_map(ElementRef::wrap)
            .find(|e| e.value().name() == "math")
    };
    if let Some(alt) = math.and_then(|m| m.value().attr("alttext")) {
        return Some(alt.trim().to_string());
    }
    if let Some(tex) = math
        .and_then(|m| m.value().attr("data-mw"))
        .and_then(tex_from_data_mw)
    {
        return Some(tex);
    }
    let text = math.unwrap_or(el).text().collect::<String>();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

enum Piece {
    Text(String),
    Math { tex: String, display: bool },
}

/// Pages that render formulas with client-side JavaScript leave raw TeX
/// delimiters in their HTML. Turns `$$…$$`, `\[…\]`, `\(…\)` and conservative
/// `$…$` into math runs, and a paragraph holding only display math into a
/// math block.
pub fn split_raw_tex(blocks: Vec<(u8, BlockKind)>) -> Vec<(u8, BlockKind)> {
    blocks
        .into_iter()
        .map(|(depth, kind)| match kind {
            BlockKind::Paragraph { runs } => match split_runs(runs) {
                Split::Display(tex) => (depth, BlockKind::Math { tex }),
                Split::Runs(runs) => (depth, BlockKind::Paragraph { runs }),
            },
            BlockKind::ListItem {
                ordered,
                depth: d,
                index,
                marker,
                runs,
            } => {
                let runs = match split_runs(runs) {
                    Split::Display(tex) => vec![Run::math(tex)],
                    Split::Runs(runs) => runs,
                };
                (
                    depth,
                    BlockKind::ListItem {
                        ordered,
                        depth: d,
                        index,
                        marker,
                        runs,
                    },
                )
            }
            BlockKind::Heading { level, runs } => {
                let runs = match split_runs(runs) {
                    Split::Display(tex) => vec![Run::math(tex)],
                    Split::Runs(runs) => runs,
                };
                (depth, BlockKind::Heading { level, runs })
            }
            other => (depth, other),
        })
        .collect()
}

enum Split {
    Display(String),
    Runs(Vec<Run>),
}

fn split_runs(runs: Vec<Run>) -> Split {
    if !runs
        .iter()
        .any(|r| !r.math && r.style & style::CODE == 0 && r.text.contains(['$', '\\']))
    {
        return Split::Runs(runs);
    }
    let mut out = Vec::new();
    let mut displays = Vec::new();
    for run in runs {
        if run.math || run.style & style::CODE != 0 {
            out.push(run);
            continue;
        }
        for piece in split_text(&run.text) {
            match piece {
                Piece::Text(text) => out.push(Run {
                    text,
                    ..run.clone()
                }),
                Piece::Math { tex, display } => {
                    if display {
                        displays.push(out.len());
                    }
                    out.push(Run {
                        text: tex,
                        style: 0,
                        link: run.link.clone(),
                        math: true,
                    });
                }
            }
        }
    }
    let only_display = displays.len() == 1
        && out
            .iter()
            .enumerate()
            .all(|(i, r)| i == displays[0] || (!r.math && r.text.trim().is_empty()));
    if only_display {
        return Split::Display(out.swap_remove(displays[0]).text);
    }
    out.retain(|r| r.math || !r.text.is_empty());
    Split::Runs(out)
}

fn split_text(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some((tex, display, consumed)) = match_math(rest) {
            if !plain.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut plain)));
            }
            pieces.push(Piece::Math { tex, display });
            rest = &rest[consumed..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
        // An escaped dollar is literal text.
        if ch == '\\' && rest.starts_with('$') {
            plain.pop();
            plain.push('$');
            rest = &rest[1..];
        }
    }
    if !plain.is_empty() {
        pieces.push(Piece::Text(plain));
    }
    pieces
}

fn match_math(s: &str) -> Option<(String, bool, usize)> {
    for (open, close, display) in [
        ("$$", "$$", true),
        ("\\[", "\\]", true),
        ("\\(", "\\)", false),
    ] {
        if let Some(body) = s.strip_prefix(open) {
            let end = body.find(close)?;
            let tex = normalize_tex(&body[..end]);
            if tex.is_empty() {
                return None;
            }
            return Some((tex, display, open.len() + end + close.len()));
        }
    }
    let body = s.strip_prefix('$')?;
    let end = body.find('$')?;
    let tex = &body[..end];
    let after = body[end + 1..].chars().next();
    let plausible = !tex.is_empty()
        && tex.len() <= 300
        && !tex.contains('\n')
        && !tex.starts_with(char::is_whitespace)
        && !tex.ends_with(char::is_whitespace)
        && !after.is_some_and(|c| c.is_ascii_digit())
        && (tex.contains(['\\', '^', '_', '{', '}', '='])
            || (tex.chars().count() <= 3
                && tex.chars().all(|c| c.is_ascii_alphanumeric())
                && tex.chars().next().unwrap().is_ascii_alphabetic()));
    plausible.then(|| (tex.to_string(), false, 1 + end + 1))
}
