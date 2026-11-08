use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[test]
fn xpath_subset_translates_to_css() {
    assert_eq!(to_css("article .body").unwrap(), "article .body");
    assert_eq!(
        to_css("//div[@id='content']").unwrap(),
        "div[id=\"content\"]"
    );
    assert_eq!(
        to_css("//div[contains(@class, 'post-body')]").unwrap(),
        "div.post-body"
    );
    assert_eq!(
        to_css("//div[contains(concat(' ',normalize-space(@class),' '),' entry ')]").unwrap(),
        "div.entry"
    );
    assert_eq!(
        to_css("//article//p | //aside").unwrap(),
        "article p, aside"
    );
    assert_eq!(
        to_css("//div[@class='a']/span[2]").unwrap(),
        "div[class=\"a\"] > span:nth-of-type(2)"
    );
    assert_eq!(
        to_css("//*[@itemprop='articleBody']").unwrap(),
        "[itemprop=\"articleBody\"]"
    );
    assert!(to_css("//div[text()='x']").is_none());
}

#[test]
fn rule_packs_parse_and_match_most_specific_domain() {
    let pack = RulePack::parse_toml(
        r#"
        name = "test"
        version = 1
        [[site]]
        domains = [".example.com"]
        strategies = ["archive"]
        body = ["//div[@id='a']"]
        [[site]]
        domains = ["news.example.com"]
        strategies = ["headers"]
        referer = "https://news.example.com/"
        cookies = { session = "abc" }
        "#,
    )
    .unwrap();
    assert_eq!(pack.sites[0].body, ["div[id=\"a\"]"]);
    assert_eq!(
        pack.rule_for("www.news.example.com").unwrap().strategies,
        ["headers"]
    );
    assert_eq!(
        pack.rule_for("blog.example.com").unwrap().strategies,
        ["archive"]
    );
    assert!(pack.rule_for("example.com").is_none());
    assert!(pack.rule_for("notexample.com").is_none());
    assert!(RulePack::parse_toml("site = 3").is_err());
}

#[test]
fn ftr_files_parse_supported_directives() {
    let rule = parse_ftr(
        "example.com",
        "# comment\nbody: //div[@class='article']\nstrip: //div[@class='ad']\nstrip_id_or_class: promo\n\
         next_page_link: //a[@rel='next']\nhttp_header(referer): https://www.example.com/\ntest_url: https://example.com/x\n",
    );
    assert_eq!(rule.body, ["div[class=\"article\"]"]);
    assert_eq!(rule.strip.len(), 2);
    assert_eq!(rule.next_page.as_deref(), Some("a[rel=\"next\"]"));
    assert_eq!(rule.referer.as_deref(), Some("https://www.example.com/"));
    assert_eq!(rule.strategies, ["headers", "cookies"]);
}

fn long_page() -> String {
    format!(
        "<html><body><article>{}</article></body></html>",
        "<p>Long enough paragraph of text.</p>".repeat(40)
    )
}

#[tokio::test]
async fn falls_through_strategies_until_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/story"))
        .and(header("referer", "https://ref.example/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string(long_page()),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/story"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string("<p>Subscribe to read</p>"),
        )
        .with_priority(5)
        .mount(&server)
        .await;
    let pack = RulePack {
        sites: vec![SiteRule {
            domains: vec!["127.0.0.1".into()],
            strategies: vec!["headers".into()],
            referer: Some("https://ref.example/".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let fetcher = Fetcher::new(pack, None).unwrap();
    let page = fetcher
        .fetch(&format!("{}/story", server.uri()), |p| p.html.len() > 500)
        .await
        .unwrap();
    assert_eq!(page.strategy, "headers");

    // Without a rule, the best (only) page is returned even if not accepted.
    let plain = Fetcher::new(RulePack::default(), None).unwrap();
    let page = plain
        .fetch(&format!("{}/story", server.uri()), |p| p.html.len() > 500)
        .await
        .unwrap();
    assert_eq!(page.strategy, "direct");
}

#[tokio::test]
async fn archive_strategy_uses_the_original_snapshot() {
    let server = MockServer::start().await;
    let target = "https://paper.example/a";
    Mock::given(method("GET"))
        .and(path("/wayback/available"))
        .and(query_param("url", target))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "archived_snapshots": {"closest": {"available": true, "timestamp": "20240101000000"}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/web/20240101000000id_/https://paper.example/a"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(long_page()),
        )
        .mount(&server)
        .await;
    let fetcher = Fetcher::new(RulePack::default(), None)
        .unwrap()
        .with_archive_base(server.uri());
    let page = fetcher.archive(&Url::parse(target).unwrap()).await.unwrap();
    assert_eq!(page.strategy, "archive");
    assert_eq!(page.final_url, target);
}

struct FakeWebView;

#[async_trait::async_trait]
impl WebRenderer for FakeWebView {
    async fn render(&self, _url: &str) -> Option<String> {
        Some("<html><body><article><h1>Rendered</h1><p>After JavaScript ran.</p></article></body></html>".into())
    }
}

#[tokio::test]
async fn client_rendered_pages_go_through_the_webview() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/book.html"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(
            "<meta charset='utf-8'>**Raw markdown** that is long enough to be accepted as is.<script src='markdeep.min.js'></script>",
        ))
        .mount(&server)
        .await;
    let url = format!("{}/book.html", server.uri());
    let with_view =
        Fetcher::new(RulePack::default(), Some(std::sync::Arc::new(FakeWebView))).unwrap();
    let page = with_view.fetch(&url, |_| true).await.unwrap();
    assert_eq!(page.strategy, "webview");
    let without = Fetcher::new(RulePack::default(), None).unwrap();
    assert_eq!(
        without.fetch(&url, |_| true).await.unwrap().strategy,
        "direct"
    );
    assert!(needs_js(
        "<html><body><div id=\"root\"></div><noscript>Please enable JavaScript</noscript></body></html>"
    ));
    assert!(!needs_js(&long_page()));
}

#[tokio::test]
async fn sign_in_prompts_go_through_the_webview_and_are_never_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/x/comments/1"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string(
            "<html><body><p>Sign up for the most real communities. Continue with phone. Continue with email.</p>\
             <ac-track san=\"xpromo|click|blocking_xpromo_nsfw_blocking\"></ac-track></body></html>",
        ))
        .mount(&server)
        .await;
    let url = format!("{}/r/x/comments/1", server.uri());
    let with_view =
        Fetcher::new(RulePack::default(), Some(std::sync::Arc::new(FakeWebView))).unwrap();
    assert_eq!(
        with_view.fetch(&url, |_| true).await.unwrap().strategy,
        "webview"
    );
    let without = Fetcher::new(RulePack::default(), None).unwrap();
    let page = without.fetch(&url, |_| true).await.unwrap();
    assert!(is_login_wall(&page.html));
    assert!(is_login_wall(
        "<configured-xpromo-modal data-xpromo-view-san=\"xpromo|view|bypassable_xpromo_nsfw_bypassable\">"
    ));
    assert!(!is_login_wall(&long_page()));
    assert!(is_bot_wall(
        "<html><head><title>Reddit - Prove your humanity</title></head></html>"
    ));
}

#[test]
fn decodes_legacy_charsets() {
    let (bytes, _, _) = encoding_rs::GBK.encode("中文内容");
    assert_eq!(decode_html(&bytes, "text/html; charset=gbk"), "中文内容");
    let mut html = b"<meta charset=\"gb2312\">".to_vec();
    html.extend_from_slice(&bytes);
    assert!(decode_html(&html, "text/html").contains("中文内容"));
}
