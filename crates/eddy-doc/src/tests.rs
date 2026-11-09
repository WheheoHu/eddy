use eddy_model::{BlockKind, Document, MediaKind, Position, TokenClass, style};

use super::*;

fn doc(html: &str) -> Document {
    html_to_document(
        html,
        &ConvertOptions {
            base_url: Some("https://example.com/post/1"),
            title: Some("Hello World"),
        },
    )
}

fn kinds(d: &Document) -> Vec<&'static str> {
    d.blocks.iter().map(|b| b.kind.tag()).collect()
}

#[test]
fn keeps_semantic_structure_and_drops_unsafe_markup() {
    let d = doc(r#"
        <h1>Hello World</h1>
        <p>Intro <strong>bold</strong> and <em>it</em> <a href="/x">link</a> <a href="javascript:alert(1)">bad</a></p>
        <script>alert(1)</script><style>p{}</style>
        <form><input value="x"></form>
        <h2>Section</h2>
        <hr>
    "#);
    assert_eq!(kinds(&d), ["paragraph", "heading", "rule"]);
    let BlockKind::Paragraph { runs } = &d.blocks[0].kind else {
        panic!()
    };
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "Intro bold and it link bad");
    assert!(
        runs.iter()
            .any(|r| r.text == "bold" && r.style & style::BOLD != 0)
    );
    assert!(
        runs.iter()
            .any(|r| r.text == "link" && r.link.as_deref() == Some("https://example.com/x"))
    );
    assert!(
        runs.iter()
            .all(|r| r.link.as_deref().is_none_or(|l| l.starts_with("https://")))
    );
}

#[test]
fn archive_references_resolve_only_inside_an_archive() {
    let html = r#"<p><a href="/OEBPS/ch2.xhtml#n1">note</a></p><img src="/OEBPS/img/a%20b.png" alt="A">
        <p><a href="epub://other/x.xhtml">elsewhere</a></p>"#;
    let d = html_to_document(
        html,
        &ConvertOptions {
            base_url: Some("epub://abc/OEBPS/ch1.xhtml"),
            title: None,
        },
    );
    let BlockKind::Paragraph { runs } = &d.blocks[0].kind else {
        panic!()
    };
    assert_eq!(
        runs[0].link.as_deref(),
        Some("epub://abc/OEBPS/ch2.xhtml#n1")
    );
    assert!(
        matches!(&d.blocks[1].kind, BlockKind::Image { src, .. } if src == "epub://abc/OEBPS/img/a%20b.png")
    );
    // A web page cannot point into a book.
    let web =
        doc(r#"<p><a href="epub://abc/OEBPS/ch2.xhtml">x</a></p><img src="epub://abc/a.png">"#);
    assert!(
        web.blocks
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Image { .. }))
    );
    let BlockKind::Paragraph { runs } = &web.blocks[0].kind else {
        panic!()
    };
    assert!(runs.iter().all(|r| r.link.is_none()));
}

#[test]
fn pdf_images_resolve_only_inside_their_pdf() {
    let d = html_to_document(
        r#"<figure><img src="2/7" width="900" height="600"><figcaption>Figure 1</figcaption></figure>"#,
        &ConvertOptions {
            base_url: Some("pdf://abc/"),
            title: None,
        },
    );
    assert!(
        matches!(&d.blocks[0].kind, BlockKind::Image { src, .. } if src == "pdf://abc/2/7"),
        "{:?}",
        d.blocks
    );
    assert_eq!(d.blocks[0].plain_text(), "Figure 1");
    let web = doc(r#"<p>x</p><img src="pdf://abc/2/7">"#);
    assert!(
        web.blocks
            .iter()
            .all(|b| !matches!(b.kind, BlockKind::Image { .. }))
    );
    // Nor can a book reach into a PDF.
    let book = html_to_document(
        r#"<img src="pdf://abc/2/7">"#,
        &ConvertOptions {
            base_url: Some("epub://book/ch1.xhtml"),
            title: None,
        },
    );
    assert!(book.blocks.is_empty());
}

#[test]
fn nested_lists_quotes_and_continuations() {
    let d = doc(
        "<ol start=3><li>one</li><li>two<ul><li>inner</li></ul><p>more</p></li></ol><blockquote><p>q</p><blockquote>qq</blockquote></blockquote>",
    );
    let items: Vec<_> = d
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::ListItem {
                ordered,
                depth,
                index,
                marker,
                ..
            } => Some((*ordered, *depth, *index, *marker, b.plain_text())),
            _ => None,
        })
        .collect();
    assert_eq!(
        items,
        vec![
            (true, 0, 3, true, "one".into()),
            (true, 0, 4, true, "two".into()),
            (false, 1, 1, true, "inner".into()),
            (true, 0, 4, false, "more".into()),
        ]
    );
    let quotes: Vec<_> = d
        .blocks
        .iter()
        .filter(|b| b.quote_depth > 0)
        .map(|b| (b.quote_depth, b.plain_text()))
        .collect();
    assert_eq!(quotes, vec![(1, "q".into()), (2, "qq".into())]);
}

#[test]
fn code_blocks_keep_whitespace_and_get_highlighted() {
    let d = doc(
        "<pre><code class=\"language-rust\">fn main() {\n    // hi\n    let x = \"s\";\n}</code></pre>",
    );
    let BlockKind::Code {
        language,
        text,
        highlights,
    } = &d.blocks[0].kind
    else {
        panic!("{:?}", d.blocks)
    };
    assert_eq!(language.as_deref(), Some("rust"));
    assert!(text.contains("\n    // hi\n"));
    assert!(highlights.iter().any(|h| h.class == TokenClass::Comment));
    assert!(
        highlights
            .iter()
            .any(|h| h.class == TokenClass::Keyword || h.class == TokenClass::Type)
    );
    assert!(highlights.iter().any(|h| h.class == TokenClass::String));
}

#[test]
fn formulas_from_katex_mathjax_and_raw_tex() {
    let katex = r#"<p>Energy <span class="katex"><span class="katex-mathml"><math><semantics><mrow><mi>E</mi></mrow><annotation encoding="application/x-tex">E=mc^2</annotation></semantics></math></span><span class="katex-html" aria-hidden="true">E=mc2</span></span> holds.</p>
        <span class="katex-display"><span class="katex"><span class="katex-mathml"><math><annotation encoding="application/x-tex">\int_0^1 f(x)\,dx</annotation></math></span></span></span>
        <p><script type="math/tex">\alpha</script> and <span class="MathJax_Preview">a</span><script type="math/tex; mode=display">\sum_i x_i</script></p>
        <p>Raw $x^2$ inline, costs $5 and $10, and \(\beta\).</p>
        <p>$$L_o = \int L_i \cos\theta$$</p>"#;
    let d = doc(katex);
    let BlockKind::Paragraph { runs } = &d.blocks[0].kind else {
        panic!()
    };
    assert!(runs.iter().any(|r| r.math && r.text == "E=mc^2"));
    assert!(!d.blocks[0].plain_text().contains("E=mc2 "));
    assert!(matches!(&d.blocks[1].kind, BlockKind::Math { tex } if tex == "\\int_0^1 f(x)\\,dx"));
    assert!(
        d.blocks
            .iter()
            .any(|b| matches!(&b.kind, BlockKind::Math { tex } if tex == "\\sum_i x_i"))
    );
    let raw = d
        .blocks
        .iter()
        .find(|b| b.plain_text().starts_with("Raw"))
        .unwrap();
    let BlockKind::Paragraph { runs } = &raw.kind else {
        panic!()
    };
    let math: Vec<_> = runs
        .iter()
        .filter(|r| r.math)
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(math, ["x^2", "\\beta"]);
    assert!(raw.plain_text().contains("costs $5 and $10"));
    assert!(
        matches!(&d.blocks.last().unwrap().kind, BlockKind::Math { tex } if tex == "L_o = \\int L_i \\cos\\theta")
    );
}

#[test]
fn display_math_inside_text_splits_the_paragraph() {
    let d = doc(
        r"<p>The estimator is \[F_N = \frac{1}{N}\sum f(X_i)\], where N is the count.</p><dl><dd>\[L = \int f\],</dd></dl>",
    );
    let kinds: Vec<_> = d.blocks.iter().map(|b| b.kind.tag()).collect();
    assert_eq!(kinds, ["paragraph", "math", "paragraph", "math"]);
    assert_eq!(d.blocks[0].plain_text(), "The estimator is");
    assert_eq!(d.blocks[2].plain_text(), ", where N is the count.");
    assert!(d.blocks.iter().all(|b| {
        b.kind
            .runs()
            .is_none_or(|r| r.iter().all(|r| r.style < (1 << 15)))
    }));
}

#[test]
fn images_figures_tables_and_media() {
    let d = doc(r#"
        <figure><img data-src="/a.png" src="data:image/gif;base64,R0" alt="A" width="640"><figcaption>Caption <em>here</em></figcaption></figure>
        <img src="https://t.example/pixel.gif" width="1" height="1">
        <img srcset="/s.png 320w, /l.png 1280w" src="placeholder.png">
        <table><thead><tr><th>k</th><th>v</th></tr></thead><tbody><tr><td>a</td><td colspan="2">b</td></tr></tbody></table>
        <table><tr><td><p>layout only</p></td></tr></table>
        <video poster="/p.jpg"><source src="/v.mp4"></video>
        <iframe src="https://www.youtube.com/embed/xyz" title="yt"></iframe>
    "#);
    let BlockKind::Image {
        src,
        alt,
        width,
        caption,
        ..
    } = &d.blocks[0].kind
    else {
        panic!("{:?}", d.blocks[0])
    };
    assert_eq!(src, "https://example.com/a.png");
    assert_eq!(alt, "A");
    assert_eq!(*width, Some(640));
    assert_eq!(eddy_model::runs_plain_text(caption), "Caption here");
    assert!(
        matches!(&d.blocks[1].kind, BlockKind::Image { src, .. } if src == "https://example.com/l.png")
    );
    let BlockKind::Table { rows } = &d.blocks[2].kind else {
        panic!("{:?}", d.blocks[2])
    };
    assert_eq!(rows.len(), 2);
    assert!(rows[0].cells[0].header);
    assert_eq!(rows[1].cells[1].colspan, 2);
    assert!(matches!(&d.blocks[3].kind, BlockKind::Paragraph { .. }));
    assert!(
        matches!(&d.blocks[4].kind, BlockKind::Media { media: MediaKind::Video, src, poster: Some(_), .. } if src == "https://example.com/v.mp4")
    );
    assert!(matches!(
        &d.blocks[5].kind,
        BlockKind::Media {
            media: MediaKind::Embed,
            ..
        }
    ));
}

/// A comment thread laid out with tables, the way Hacker News does it:
/// each comment is a one-row table whose last cell holds the paragraphs.
/// It reads as paragraphs, one comment after another, not as a grid.
#[test]
fn layout_tables_read_as_their_paragraphs() {
    let d = doc(r#"
        <table class="comment-tree">
          <tr><td>
            <table><tr>
              <td class="ind" indent="0"><img src="s.gif" height="1" width="0"></td>
              <td class="votelinks"><a href="vote?id=1"><div class="votearrow"></div></a></td>
              <td class="default">
                <div class="comhead"><a href="user?id=ann">ann</a> <span>2 hours ago</span></div><br>
                <div class="comment"><span class="commtext"><p>First point.</p><p>Second point.</p></span></div>
              </td>
            </tr></table>
          </td></tr>
          <tr><td>
            <table><tr>
              <td class="ind" indent="1"><img src="s.gif" height="1" width="40"></td>
              <td class="votelinks"></td>
              <td class="default"><div class="comment"><span class="commtext">A reply.</span></div></td>
            </tr></table>
          </td></tr>
        </table>
        <table><tr><td>Left column</td><td>Right column</td></tr></table>
        <table><tr><td>k1</td><td>v1</td></tr><tr><td>k2</td><td>v2</td></tr></table>
    "#);
    let texts: Vec<String> = d
        .blocks
        .iter()
        .map(|b| match &b.kind {
            BlockKind::Paragraph { runs } => eddy_model::runs_plain_text(runs),
            BlockKind::Table { rows } => format!("table {}x{}", rows.len(), rows[0].cells.len()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        texts,
        [
            "ann 2 hours ago",
            "First point.",
            "Second point.",
            "A reply.",
            // One row: layout, but its cells stay apart.
            "Left column",
            "Right column",
            // A real grid of plain values.
            "table 2x2",
        ]
    );
}

#[test]
fn block_ids_are_stable_across_unrelated_edits() {
    let a = doc("<p>First paragraph.</p><p>Second paragraph.</p><p>Repeated</p><p>Repeated</p>");
    let b = doc(
        "<p>First paragraph, edited.</p><p>Second paragraph.</p><p>Repeated</p><p>Repeated</p>",
    );
    assert_ne!(a.blocks[0].id, b.blocks[0].id);
    assert_eq!(a.blocks[1].id, b.blocks[1].id);
    assert_ne!(a.blocks[2].id, a.blocks[3].id);
    assert!(a.blocks[3].id.ends_with("-2"));
}

#[test]
fn heading_permalinks_are_dropped() {
    let d = doc(
        "<h2>Argument Passing<a class=\"headerlink\" href=\"#arg\">¶</a></h2><h3>Install <a href=\"#install\">#</a></h3><p>See <a href=\"#x\">section 2</a>.</p>",
    );
    assert_eq!(d.blocks[0].plain_text(), "Argument Passing");
    assert_eq!(d.blocks[1].plain_text(), "Install");
    assert_eq!(d.blocks[2].plain_text(), "See section 2.");
}

#[test]
fn bare_text_and_br_become_paragraph_text() {
    let d = doc("Line one<br>line two\n\n  spaced   out");
    assert_eq!(d.blocks.len(), 1);
    assert_eq!(d.blocks[0].plain_text(), "Line one\nline two spaced out");
}

#[test]
fn translation_segments_cover_text_blocks() {
    let d = doc(
        "<h2>Title</h2><p>Body with $x$ math.</p><figure><img src='/i.png'><figcaption>Cap</figcaption></figure><pre>code</pre><p>123</p>",
    );
    let segs = translation_segments(&d);
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[1].text, "Body with $x$ math.");
    assert!(segs[2].id.ends_with("#caption"));
}

#[test]
fn locator_round_trip_and_remap_across_versions() {
    let feed = doc("<p>Short teaser about rendering.</p><p>Read more…</p>");
    let full = doc(
        "<p>Intro added by the full text.</p><p>Short teaser about rendering.</p><p>The path tracer accumulates radiance over many samples per pixel.</p><p>End.</p>",
    );
    let target = &full.blocks[2];
    let loc = locator_at(&full, "full", &target.id, 9).unwrap();
    assert!(loc.quote.exact.starts_with("tracer"));
    assert!(loc.progression > 0.2 && loc.progression < 0.9);

    // Same version: unchanged.
    assert_eq!(remap_locator(&full, "full", &loc), loc);

    // Block missing in another version, but quote present after an edit.
    let edited = doc(
        "<p>Intro.</p><p>Note: The path tracer accumulates radiance over many samples per pixel.</p>",
    );
    let moved = remap_locator(&edited, "edited", &loc);
    let Position::Block { block_id, offset } = &moved.position else {
        panic!()
    };
    assert_eq!(block_id, &edited.blocks[1].id);
    assert_eq!(*offset, 15);

    // Feed version anchor carried into full text by block ID.
    let teaser = locator_at(&feed, "feed", &feed.blocks[0].id, 6).unwrap();
    let carried = remap_locator(&full, "full", &teaser);
    assert!(
        matches!(&carried.position, Position::Block { block_id, offset: 6 } if block_id == &full.blocks[1].id)
    );
}

#[test]
fn offsets_are_utf16_and_inline_math_counts_as_one_unit() {
    let d = doc(
        "<p>𝛼 then <span class=\"katex\"><annotation encoding=\"application/x-tex\">x</annotation></span> then 中文</p>",
    );
    let text = d.blocks[0].layout_text();
    assert_eq!(text, "𝛼 then \u{FFFC} then 中文");
    assert_eq!(eddy_model::utf16_len(&text), 2 + 6 + 1 + 6 + 2);
    let loc = locator_at(&d, "h", &d.blocks[0].id, 9).unwrap();
    assert!(loc.quote.exact.starts_with(" then 中文"));
}
