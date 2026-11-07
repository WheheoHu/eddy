use eddy_doc::{ConvertOptions, html_to_document};
use eddy_fetch::SiteRule;

use super::*;

fn article_page() -> String {
    let paragraphs: String = (0..12)
        .map(|i| format!("<p>Paragraph {i} explains how a path tracer integrates incoming radiance over the hemisphere, sample by sample, until the estimate converges.</p>"))
        .collect();
    format!(
        r#"<html><head><title>Path Tracing Notes | Blog</title><meta property="og:title" content="Path Tracing Notes"></head>
        <body><nav><a href="/">Home</a><a href="/about">About</a></nav>
        <div class="ad">Buy now</div>
        <article><h1>Path Tracing Notes</h1>{paragraphs}
        <p>The estimator is <script type="math/tex">L_o = \int L_i \cos\theta</script> as usual.</p>
        <script type="math/tex; mode=display">\frac{{1}}{{N}} \sum f(x_i)</script>
        </article>
        <footer>Copyright</footer></body></html>"#
    )
}

#[test]
fn readability_extracts_article_and_keeps_math() {
    let page = article_page();
    let out = extract(&page, "https://blog.example/posts/pt", None).unwrap();
    assert_eq!(out.title.as_deref(), Some("Path Tracing Notes"));
    assert!(out.text_len > 1000);
    assert!(!out.content_html.contains("Copyright"));
    let doc = html_to_document(
        &out.content_html,
        &ConvertOptions {
            base_url: Some("https://blog.example/posts/pt"),
            title: out.title.as_deref(),
        },
    );
    let text = doc.plain_text();
    assert!(text.contains("$L_o = \\int L_i \\cos\\theta$"), "{text}");
    assert!(doc.blocks.iter().any(
        |b| matches!(&b.kind, eddy_model::BlockKind::Math { tex } if tex.contains("\\frac{1}{N}"))
    ));
}

#[test]
fn wikipedia_style_mathml_survives_readability() {
    let filler: String = (0..10).map(|i| format!("<p>Filler paragraph {i} with enough words to look like an article body for readability scoring.</p>")).collect();
    let page = format!(
        r#"<html><head><title>Rendering equation</title></head><body><div id="content"><h1>Rendering equation</h1>{filler}
        <p>The equation <span class="mwe-math-element"><span class="mwe-math-mathml-inline mwe-math-mathml-a11y" style="display: none;"><math xmlns="http://www.w3.org/1998/Math/MathML" alttext="{{\displaystyle L_o}}"><semantics><mrow><msub><mi>L</mi><mi>o</mi></msub></mrow><annotation encoding="application/x-tex">{{\displaystyle L_o}}</annotation></semantics></math></span><img src="https://wikimedia.org/api/rest_v1/media/math/render/svg/abc" class="mwe-math-fallback-image-inline" alt="{{\displaystyle L_o}}"></span> is outgoing radiance.</p>
        <math display="block" alttext="{{\displaystyle \int_\Omega f}}"><mrow><mo>∫</mo></mrow></math>
        <p>Parsoid output: <math class="mwe-math-element" data-mw='{{"name":"math","body":{{"extsrc":"x &lt; y&apos;"}}}}'><mi>x</mi></math> holds.</p>
        </div></body></html>"#
    );
    let out = extract(
        &page,
        "https://en.wikipedia.org/wiki/Rendering_equation",
        None,
    )
    .unwrap();
    assert!(!out.content_html.contains("mwe-math-fallback"));
    let doc = html_to_document(&out.content_html, &ConvertOptions::default());
    let para = doc
        .blocks
        .iter()
        .find(|b| b.plain_text().starts_with("The equation"))
        .unwrap();
    assert_eq!(
        para.plain_text(),
        "The equation $L_o$ is outgoing radiance."
    );
    assert!(doc.blocks.iter().any(
        |b| matches!(&b.kind, eddy_model::BlockKind::Math { tex } if tex == "\\int_\\Omega f")
    ));
    let parsoid = doc
        .blocks
        .iter()
        .find(|b| b.plain_text().starts_with("Parsoid"))
        .unwrap();
    assert_eq!(parsoid.plain_text(), "Parsoid output: $x < y'$ holds.");
}

#[test]
fn site_rules_override_readability() {
    let page = article_page();
    let rule = SiteRule {
        body: vec!["article".into()],
        strip: vec![".ad".into(), "article p:first-of-type".into()],
        title: Some("h1".into()),
        ..Default::default()
    };
    let out = extract(&page, "https://blog.example/posts/pt", Some(&rule)).unwrap();
    assert!(!out.content_html.contains("Paragraph 0 "));
    assert!(out.content_html.contains("Paragraph 1 "));
    assert_eq!(out.title.as_deref(), Some("Path Tracing Notes"));
}

#[test]
fn truncation_detection() {
    let teaser = html_to_document(
        "<p>A short teaser about the new renderer…</p>",
        &ConvertOptions::default(),
    );
    assert!(truncation_hint(&teaser));
    let long_text =
        "<p>Full body sentence with enough words to count as real content.</p>".repeat(20);
    let full = html_to_document(&long_text, &ConvertOptions::default());
    assert!(!truncation_hint(&full));
    let read_more = html_to_document(
        &format!("{long_text}<p><a href='/x'>Continue reading →</a></p>"),
        &ConvertOptions::default(),
    );
    assert!(truncation_hint(&read_more));
    assert!(is_truncated(200, 3000));
    assert!(!is_truncated(2900, 3000));
}

#[test]
fn detects_a_docs_table_of_contents() {
    let toc: String = (1..=8)
        .map(|i| format!("<li><a href=\"/book/ch{i:02}.html\">Chapter {i}</a></li>"))
        .collect();
    let page = format!(
        r#"<html><head><title>The Book</title></head><body>
        <header><a href="/">Home</a><a href="https://twitter.com/x">Twitter</a></header>
        <nav class="sidebar"><ol>{toc}<li><a href="/book/ch01.html#intro">Intro</a></li></ol></nav>
        <main><p>Welcome. See <a href="/blog/news">news</a>.</p></main></body></html>"#
    );
    let series = detect_series(&page, "https://docs.example/book/index.html").unwrap();
    assert_eq!(series.title, "The Book");
    assert_eq!(series.chapters.len(), 8);
    assert_eq!(
        series.chapters[0].url,
        "https://docs.example/book/ch01.html"
    );
    assert_eq!(series.chapters[7].title, "Chapter 8");
    assert!(detect_series(&article_page(), "https://blog.example/posts/pt").is_none());
}
