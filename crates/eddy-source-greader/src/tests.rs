use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[test]
fn normalizes_subpath_ports_and_api_endpoints() {
    assert_eq!(normalize_base("https://rss.example.com/").unwrap(), "https://rss.example.com/api/greader.php");
    assert_eq!(normalize_base("https://rss.example.com/reader/").unwrap(), "https://rss.example.com/reader/api/greader.php");
    assert_eq!(normalize_base("https://rss.example.com/p/api/greader.php/").unwrap(), "https://rss.example.com/p/api/greader.php");
    assert_eq!(normalize_base("https://rss.example.com:43966/").unwrap(), "https://rss.example.com:43966/api/greader.php");
    assert_eq!(normalize_base("http://192.168.1.5:8080/").unwrap(), "http://192.168.1.5:8080/api/greader.php");
    for bad in ["ftp://rss.example.com", "https://user:pass@example.com", "https://example.com/?key=secret", "https://example.com/#x", "not a url"] {
        assert!(normalize_base(bad).is_err(), "{bad}");
    }
}

#[test]
fn history_paths_query_the_selected_stream_without_a_date_cutoff() {
    for (view, stream) in [("all", READING_LIST), ("starred", STARRED), ("feed/42", "feed/42")] {
        let path = history_path(view, false, Some("older+page&token")).unwrap();
        let url = url::Url::parse(&format!("https://rss.example.com/{path}")).unwrap();
        assert_eq!(url.path(), format!("/stream/contents/{stream}"));
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q.get("c").map(String::as_str), Some("older+page&token"));
        assert_eq!(q.get("n").map(String::as_str), Some("100"));
        assert!(!q.contains_key("ot") && !q.contains_key("nt") && !q.contains_key("xt"));
    }
    assert!(history_path("feed/42", true, None).unwrap().contains("xt=user%2F-%2Fstate%2Fcom.google%2Fread"));
    assert!(history_path("unread", false, None).unwrap().contains("xt="));
    for view in ["feed/42?xt=evil", "feed/../all", "https://evil.example", "unknown", "feed/"] {
        assert!(history_path(view, false, None).is_err(), "{view}");
    }
    assert_eq!(history_scope("unread", false), history_scope("all", true));
}

#[test]
fn continuation_tokens_must_advance() {
    assert_eq!(validate_continuation(Some("c1"), None).unwrap(), Some("c1".into()));
    assert_eq!(validate_continuation(None, Some("c1")).unwrap(), None);
    assert!(validate_continuation(Some("c1"), Some("c1")).is_err());
    assert!(validate_continuation(Some(""), None).is_err());
    assert!(validate_continuation(Some(&"x".repeat(2000)), None).is_err());
}

#[test]
fn item_ids_normalize_to_hex() {
    assert_eq!(normalize_item_id("tag:google.com,2005:reader/item/00000000000000ff").unwrap(), "00000000000000ff");
    assert_eq!(normalize_item_id("255").unwrap(), "00000000000000ff");
    assert_eq!(normalize_item_id("-1").unwrap(), "ffffffffffffffff");
    assert!(normalize_item_id("abc").is_none());
}

fn client(server: &MockServer) -> GReaderClient {
    GReaderClient::new(Credentials { base_url: server.uri(), username: "me".into(), password: "secret".into() }).unwrap()
}

async fn mount_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .and(body_string_contains("Email=me"))
        .respond_with(ResponseTemplate::new(200).set_body_string("SID=x\nAuth=tok123\n"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn logs_in_and_reads_subscriptions_ids_and_contents() {
    let server = MockServer::start().await;
    mount_login(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/subscription/list"))
        .and(header("Authorization", "GoogleLogin auth=tok123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "subscriptions": [{"id": "feed/1", "title": "Blog", "categories": [{"id": "user/-/label/Tech", "label": "Tech"}], "htmlUrl": "https://blog.example"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/stream/items/ids"))
        .and(query_param("xt", READ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"itemRefs": [{"id": "255"}, {"id": "16"}]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/stream/items/contents"))
        .and(body_string_contains("i=tag%3Agoogle.com%2C2005%3Areader%2Fitem%2F00000000000000ff"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": [{
            "id": "tag:google.com,2005:reader/item/00000000000000ff",
            "title": "Post", "published": 1700000000, "author": "A",
            "categories": ["user/-/state/com.google/reading-list", "user/-/state/com.google/starred"],
            "alternate": [{"href": "https://blog.example/post"}],
            "summary": {"content": "<p>Hi</p>"},
            "origin": {"streamId": "feed/1", "title": "Blog"}
        }]})))
        .mount(&server)
        .await;

    let c = client(&server);
    let subs = c.subscriptions().await.unwrap();
    assert_eq!(subs[0].category.as_deref(), Some("Tech"));
    let ids = c.item_ids(READING_LIST, Some(READ), 10_000).await.unwrap();
    assert_eq!(ids, ["00000000000000ff", "0000000000000010"]);
    let items = c.contents(&ids[..1]).await.unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!((item.title.as_str(), item.read, item.starred), ("Post", false, true));
    assert_eq!(item.published_ms, 1_700_000_000_000);
    assert_eq!(item.url.as_deref(), Some("https://blog.example/post"));
    assert_eq!(item.html, "<p>Hi</p>");
}

#[tokio::test]
async fn edit_tag_sends_token_and_long_ids() {
    let server = MockServer::start().await;
    mount_login(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/greader.php/reader/api/0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("edit-token\n"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/reader/api/0/edit-tag"))
        .and(body_string_contains("T=edit-token"))
        .and(body_string_contains("a=user%2F-%2Fstate%2Fcom.google%2Fread"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .expect(1)
        .mount(&server)
        .await;
    client(&server).edit_tag(&["00000000000000ff".into()], READ, true).await.unwrap();
}

#[tokio::test]
async fn redirects_and_auth_failures_are_reported_without_following() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "https://other.example.com/collect"))
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server).login().await.unwrap_err();
    assert!(matches!(err, GReaderError::Redirect(302)), "{err:?}");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/greader.php/accounts/ClientLogin"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Error=BadAuthentication"))
        .mount(&server)
        .await;
    let err = client(&server).subscriptions().await.unwrap_err();
    assert!(matches!(err, GReaderError::Auth(401)));
    assert!(!format!("{err} {:?}", Credentials { base_url: "x".into(), username: "u".into(), password: "secret".into() }).contains("secret"));
}
