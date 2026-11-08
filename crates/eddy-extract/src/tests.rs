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
fn mathjax_v2_rendered_output_is_not_duplicated() {
    let filler: String = (0..10).map(|i| format!("<p>Filler paragraph {i} with enough words to look like an article body for scoring purposes.</p>")).collect();
    let page = format!(
        r#"<html><head><title>T</title></head><body><article>{filler}
        <p>Reflectance <span class="MathJax_Preview"></span><span class="mjx-chtml MathJax_CHTML" id="MathJax-Element-1-Frame"><span class="mjx-mi">R</span></span><script type="math/tex" id="MathJax-Element-1">R</script> scatters.</p>
        </article></body></html>"#
    );
    let out = extract(&page, "https://raytracing.example/book.html", None).unwrap();
    let doc = html_to_document(&out.content_html, &ConvertOptions::default());
    assert!(
        doc.blocks
            .iter()
            .any(|b| b.plain_text() == "Reflectance $R$ scatters."),
        "{:?}",
        doc.plain_text()
    );
}

#[test]
fn titles_drop_site_names() {
    let body: String = (0..12).map(|i| format!("<p>Ownership paragraph {i} explains moves, borrows and lifetimes in enough words to count.</p>")).collect();
    let page = format!(
        "<html><head><title>What is Ownership? - The Rust Programming Language</title></head><body><div class='menu'><h1 class='menu-title'>The Rust Programming Language</h1></div><main><h2>What Is Ownership?</h2>{body}</main></body></html>"
    );
    assert_eq!(
        extract(&page, "https://doc.rust-lang.org/book/ch04-01.html", None)
            .unwrap()
            .title
            .as_deref(),
        Some("What is Ownership?")
    );
    let blog = format!(
        "<html><head><title>My Post | Some Blog</title></head><body><article><h1>My Post</h1>{body}</article></body></html>"
    );
    assert_eq!(
        extract(&blog, "https://blog.example/p", None)
            .unwrap()
            .title
            .as_deref(),
        Some("My Post")
    );
}

#[test]
fn wikipedia_block_wrapper_marks_display_math() {
    let filler: String = (0..10).map(|i| format!("<p>Filler paragraph {i} with enough words to look like an article body for scoring.</p>")).collect();
    let long_json = "x".repeat(2000);
    let page = format!(
        r#"<html><head><title>Eq</title></head><body><div id="content"><h1>Eq</h1>{filler}
        <p>Inline <span class="mwe-math-element mwe-math-element-inline" data-mw='{{"a":"{long_json}"}}'><span class="mwe-math-mathml-a11y" style="display: none;"><math alttext="{{\displaystyle a}}"><mi>a</mi></math></span></span> here.</p>
        <dl><dd><span class="mwe-math-element mwe-math-element-block" data-mw='{{"a":"{long_json}"}}'><span class="mwe-math-mathml-a11y" style="display: none;"><math alttext="{{\displaystyle L=\int f}}"><mi>L</mi></math></span></span></dd></dl>
        </div></body></html>"#
    );
    let out = extract(&page, "https://en.wikipedia.org/wiki/Eq", None).unwrap();
    let doc = html_to_document(&out.content_html, &ConvertOptions::default());
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(&b.kind, eddy_model::BlockKind::Math { tex } if tex == "L=\\int f"))
    );
    assert!(
        doc.blocks
            .iter()
            .any(|b| b.plain_text() == "Inline $a$ here.")
    );
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
    let picture = html_to_document(
        "<p>Rope practice tonight</p><p><img src='https://i.example/a.jpg'></p>",
        &ConvertOptions::default(),
    );
    assert!(!truncation_hint(&picture));
    let picture_teaser = html_to_document(
        "<p><img src='https://i.example/a.jpg'></p><p>The first lines of the review…</p>",
        &ConvertOptions::default(),
    );
    assert!(truncation_hint(&picture_teaser));
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
