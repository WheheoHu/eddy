use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::client::{UserPart, build_http_client, call_stage_model, extract_json_object};
use crate::epub_builder::build_epub_bytes;
use crate::media::parse_vtt_or_srt;
use crate::models::{ArticleBlock, ArticleDocument, ArticleSection};
use crate::redact::{REDACTED, redact_text};
use crate::settings::{ProtocolKind, ResolvedProtocol, StageEndpoint};
use crate::toolchain::{is_video_file_path, is_video_url};

#[test]
fn redacts_keys_and_query_tokens() {
    let raw = "Authorization: Bearer sk-secret999 x-goog-api-key: my-station-key https://a.com/x?signature=sig123";
    let safe = redact_text(raw, &["my-station-key"]);
    assert!(!safe.contains("sk-secret999"));
    assert!(!safe.contains("my-station-key"));
    assert!(!safe.contains("sig123"));
    assert!(safe.contains(REDACTED));
}

#[test]
fn protocol_auto_detection_and_url_normalization() {
    let qwen = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions".into(),
        model: "qwen3-asr-flash".into(),
        api_key: "sk-test".into(),
    };
    assert_eq!(qwen.resolved_protocol(), ResolvedProtocol::OpenAi);
    assert_eq!(
        qwen.normalized_openai_base_url().unwrap(),
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );

    let relay_gemini = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: "https://relay.example.com/v1beta".into(),
        model: "gemini-3.8-flash".into(),
        api_key: "sk-relay".into(),
    };
    assert_eq!(relay_gemini.resolved_protocol(), ResolvedProtocol::Gemini);
    assert_eq!(
        relay_gemini.gemini_generate_content_url().unwrap(),
        "https://relay.example.com/v1beta/models/gemini-3.8-flash:generateContent"
    );
}

#[test]
fn repairs_missing_type_key_json_and_strips_thought() {
    let broken = r#"<think>internal</think>```json
    {
      "title": "Watch Chapter",
      "blocks": [
        {"subheading", "text": "Heart Sensor"},
        {"type": "paragraph", "text": "Details",}
      ]
    }
    ```"#;
    let parsed = extract_json_object(broken).unwrap();
    assert_eq!(parsed["blocks"][0]["type"], "subheading");
    assert_eq!(parsed["blocks"][1]["text"], "Details");
}

#[test]
fn detects_video_urls_and_files() {
    assert!(is_video_url("https://www.youtube.com/watch?v=2OeT5jKtyvQ"));
    assert!(is_video_url("https://www.bilibili.com/video/BV1xx411c7mD"));
    assert!(!is_video_url("https://lwn.net/Articles/12345/"));
    assert!(is_video_file_path("/Users/me/Downloads/talk.mp4"));
    assert!(!is_video_file_path("/Users/me/Downloads/book.epub"));
}

#[test]
fn cleans_rolling_vtt_and_builds_multichapter_epub() {
    let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nWelcome to\n\n00:00:02.800 --> 00:00:05.500\nWelcome to\nour engine talk.\n";
    let segs = parse_vtt_or_srt(vtt);
    let joined = segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    assert_eq!(joined.matches("Welcome to").count(), 1);

    let doc = ArticleDocument {
        video_id: "yt_test".into(),
        title: "自研渲染引擎架构".into(),
        subtitle: "辐射级联解析".into(),
        author: Some("Speaker".into()),
        source_url: Some("https://www.youtube.com/watch?v=2OeT5jKtyvQ".into()),
        duration_ms: 600_000,
        executive_summary: "核心摘要".into(),
        key_takeaways: vec!["要点一".into()],
        sections: vec![
            ArticleSection {
                index: 1,
                title: "第一章 Cutout".into(),
                start_ms: 0,
                end_ms: 300_000,
                blocks: vec![ArticleBlock {
                    kind: "paragraph".into(),
                    text: "第一段正文".into(),
                    ..Default::default()
                }],
            },
            ArticleSection {
                index: 2,
                title: "第二章 Radiance Cascades".into(),
                start_ms: 300_000,
                end_ms: 600_000,
                blocks: vec![ArticleBlock {
                    kind: "math".into(),
                    tex: Some("E = mc^2".into()),
                    ..Default::default()
                }],
            },
        ],
    };

    let (epub_bytes, ch_count) = build_epub_bytes(&doc).unwrap();
    assert_eq!(ch_count, 3); // Overview + 2 chapters
    assert!(epub_bytes.len() > 500);
}

#[tokio::test]
async fn calls_gemini_token_relay_with_dual_headers_and_skips_thought() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-3.8-flash:generateContent"))
        .and(header("x-goog-api-key", "sk-station-secret"))
        .and(header("authorization", "Bearer sk-station-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "thinking...", "thought": true},
                        {"text": "{\"hello\": \"world\"}"}
                    ]
                }
            }],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        })))
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::Gemini,
        url: format!("{}/v1beta", server.uri()),
        model: "gemini-3.8-flash".into(),
        api_key: "sk-station-secret".into(),
    };
    let http = build_http_client(15).unwrap();
    let out = call_stage_model(
        &http,
        &ep,
        "Return JSON.",
        &[UserPart::Text("Hi".into())],
        0.1,
        256,
        true,
    )
    .await
    .unwrap();
    let json = extract_json_object(&out.text).unwrap();
    assert_eq!(json["hello"], "world");
}
