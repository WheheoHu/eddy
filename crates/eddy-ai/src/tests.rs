use eddy_doc::Segment;
use reqwest::header::{HeaderMap, HeaderValue};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[test]
fn model_errors_distinguish_permission_auth_quota_and_path() {
    let permission = model_failure(
        403,
        "application/json",
        r#"{"error":{"message":"The token cannot access this model","code":"model_not_allowed"}}"#,
        &[],
        None,
    );
    assert!(
        permission.contains("403")
            && permission.contains("模型权限")
            && permission.contains("model_not_allowed")
    );
    assert!(!permission.contains("余额"));
    assert!(model_failure(401, "application/json", "{}", &[], None).contains("API Key 未被接受"));
    assert!(model_failure(404, "application/json", "{}", &[], None).contains("API 路径或模型"));
    assert!(model_failure(429, "application/json", "{}", &[], None).contains("限流或配额"));
}

#[test]
fn provider_reasons_redact_keys_tokens_and_html() {
    let secret = "my+private/key";
    let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
    let raw = serde_json::json!({"error": {"message": format!("<b>Denied</b> {secret} {encoded} Bearer hidden-token sk-other-secret"), "code": secret}}).to_string();
    let result = model_failure(403, "application/json", &raw, &[secret], None);
    for hidden in [
        secret,
        encoded.as_str(),
        "hidden-token",
        "sk-other-secret",
        "<b>",
    ] {
        assert!(!result.contains(hidden), "{hidden} leaked in {result}");
    }
    assert!(result.contains("Denied") && result.contains("已隐藏"));
    let long = serde_json::json!({"message": "x".repeat(2000)}).to_string();
    assert!(
        model_failure(400, "application/json", &long, &[], None)
            .chars()
            .count()
            < 650
    );
}

#[test]
fn html_gateway_pages_produce_an_api_prefix_diagnosis_even_with_200() {
    for status in [200, 403] {
        let err = parse_completion(
            status,
            "text/html",
            "<!doctype html><html>dashboard</html>",
            &[],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("网页") && err.contains("JSON API") && err.contains("/v1"),
            "{err}"
        );
    }
    assert!(
        parse_completion(200, "application/json", "not json", &[], None)
            .unwrap_err()
            .to_string()
            .contains("未返回有效 JSON")
    );
}

#[test]
fn successful_content_is_preserved_and_structured_failures_stay_readable() {
    let expected = r#"{"translations":[{"id":"title","text":"月球"}]}"#;
    let raw = serde_json::json!({"choices": [{"message": {"content": expected}}]}).to_string();
    assert_eq!(
        parse_completion(200, "application/json", &raw, &[], None).unwrap(),
        expected
    );
    let denied = serde_json::json!({"error": {"message": "Model access denied", "code": "model_not_allowed"}}).to_string();
    assert!(
        parse_completion(403, "application/json", &denied, &[], None)
            .unwrap_err()
            .to_string()
            .contains("Model access denied")
    );
    assert!(
        parse_completion(200, "application/json", r#"{"choices":[]}"#, &[], None)
            .unwrap_err()
            .to_string()
            .contains("没有返回可用结果")
    );
}

#[test]
fn cloudflare_challenges_identify_the_gate_and_keep_a_safe_request_id() {
    let raw = "<html><head><title>Just a moment...</title></head><body>Authorization: Bearer private-token</body></html>";
    let mut headers = HeaderMap::new();
    headers.insert("cf-mitigated", HeaderValue::from_static("challenge"));
    headers.insert("cf-ray", HeaderValue::from_static("0123456789abcdef-SEA"));
    let err = parse_completion(403, "text/html", raw, &[], Some(&headers))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Cloudflare 浏览器验证")
            && err.contains("Just a moment")
            && err.contains("0123456789abcdef-SEA")
    );
    assert!(!err.contains("private-token") && !err.contains("Authorization"));
    let mut bad_ray = HeaderMap::new();
    bad_ray.insert("cf-ray", HeaderValue::from_static("private-token"));
    let hidden = model_failure(
        403,
        "text/html",
        "<title>Denied private-token</title>",
        &["private-token"],
        Some(&bad_ray),
    );
    assert!(!hidden.contains("private-token") && hidden.contains("已隐藏"));
}

#[test]
fn base_urls_default_to_v1_and_keep_explicit_prefixes() {
    assert_eq!(
        normalize_base("https://llm.example.com/v1/chat/completions").unwrap(),
        "https://llm.example.com/v1"
    );
    assert_eq!(
        normalize_base("https://llm.example.com").unwrap(),
        "https://llm.example.com/v1"
    );
    assert_eq!(
        normalize_base("https://llm.example.com:43966/").unwrap(),
        "https://llm.example.com:43966/v1"
    );
    assert_eq!(
        normalize_base("https://llm.example.com/api/v1/chat/completions/").unwrap(),
        "https://llm.example.com/api/v1"
    );
    assert_eq!(
        normalize_base("https://llm.example.com/v1beta/openai").unwrap(),
        "https://llm.example.com/v1beta/openai"
    );
    assert_eq!(
        normalize_base(&normalize_base("https://llm.example.com").unwrap()).unwrap(),
        "https://llm.example.com/v1"
    );
    assert_eq!(
        normalize_base("http://localhost:1234").unwrap(),
        "http://localhost:1234/v1"
    );
    assert!(normalize_base("https://user:pass@example.com").is_err());
    assert!(normalize_base("https://example.com/?key=secret").is_err());
}

fn segs() -> Vec<Segment> {
    vec![
        Segment {
            id: "s0".into(),
            text: "hello".into(),
        },
        Segment {
            id: "s1".into(),
            text: "world".into(),
        },
    ]
}

#[test]
fn translation_ids_are_exact_and_bad_replies_fail_closed() {
    let ok = validate_translations(
        r#"```json
{"translations":[{"id":"s1","text":"世界"},{"id":"s0","text":"你好"}]}
```"#,
        &segs(),
    )
    .unwrap();
    assert_eq!(ok["s0"], "你好");
    assert_eq!(ok["s1"], "世界");
    for bad in [
        r#"{"translations":[{"id":"s0","text":"你好"}]}"#,
        r#"{"translations":[{"id":"s0","text":"a"},{"id":"s0","text":"b"}]}"#,
        r#"{"translations":[{"id":"s0","text":""},{"id":"s1","text":"b"}]}"#,
        r#"{"translations":[{"id":"x","text":"a"},{"id":"s1","text":"b"}]}"#,
    ] {
        assert!(validate_translations(bad, &segs()).is_err(), "{bad}");
    }
}

#[test]
fn batches_respect_segment_and_size_limits() {
    let many: Vec<Segment> = (0..20)
        .map(|i| Segment {
            id: format!("s{i}"),
            text: "x".repeat(100),
        })
        .collect();
    let b = batches(&many);
    assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), [8, 8, 4]);
    let big: Vec<Segment> = (0..3)
        .map(|i| Segment {
            id: format!("s{i}"),
            text: "y".repeat(7000),
        })
        .collect();
    assert_eq!(batches(&big).len(), 3);
}

fn reply(content: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        serde_json::json!({"choices": [{"message": {"content": content.to_string()}}]}),
    )
}

#[tokio::test]
async fn openai_compatible_provider_runs_all_tasks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer key-123"))
        .and(body_string_contains("Translate each input segment into 简体中文"))
        .respond_with(reply(serde_json::json!({"translations": [{"id": "s0", "text": "你好"}, {"id": "s1", "text": "世界"}]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("You summarize articles"))
        .respond_with(reply(
            serde_json::json!({"tldr": "概述", "points": ["要点一", " ", "要点二"]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("reader's own tags"))
        .respond_with(reply(serde_json::json!({"tags": ["rendering", "Unknown"], "suggestions": ["Monte Carlo", "rendering"]})))
        .mount(&server)
        .await;

    let provider = OpenAiCompatible::new(&server.uri(), "model-x", "key-123").unwrap();
    assert!(!format!("{provider:?}").contains("key-123"));
    let t = translate_batch(&provider, &segs(), "简体中文", "")
        .await
        .unwrap();
    assert_eq!(t["s1"], "世界");
    let s = summarize(&provider, "Title", "Body", "简体中文")
        .await
        .unwrap();
    assert_eq!(
        s,
        Summary {
            tldr: "概述".into(),
            points: vec!["要点一".into(), "要点二".into()]
        }
    );
    let tags = tag(&provider, "T", "B", &["Rendering".into(), "Rust".into()])
        .await
        .unwrap();
    assert_eq!(tags.tags, ["Rendering"]);
    assert_eq!(tags.suggestions, ["Unknown", "Monte Carlo"]);
}

#[tokio::test]
async fn provider_errors_never_echo_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(serde_json::json!({"error": {"message": "Invalid key key-123"}})),
        )
        .mount(&server)
        .await;
    let provider = OpenAiCompatible::new(&server.uri(), "m", "key-123").unwrap();
    let err = summarize(&provider, "T", "B", "English")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("401") && !err.contains("key-123"), "{err}");
}

#[test]
fn target_languages_map_to_detection_codes() {
    assert_eq!(lang_code_for("简体中文"), Some("cmn"));
    assert_eq!(lang_code_for("English"), Some("eng"));
    assert_eq!(lang_code_for("日本語"), Some("jpn"));
    assert_eq!(lang_code_for("Klingon"), None);
}
