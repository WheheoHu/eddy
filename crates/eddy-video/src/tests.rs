use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::client::{
    UserPart, build_streaming_http_client, call_gemini_generate_content_with_retries,
    call_stage_model, extract_json_object, test_stage_endpoint,
};
use crate::epub_builder::build_epub_bytes;
use crate::media::parse_vtt_or_srt;
use crate::models::{ArticleBlock, ArticleDocument, ArticleSection};
use crate::redact::{REDACTED, redact_text};
use crate::settings::{
    ProtocolKind, ResolvedProtocol, StageEndpoint, VideoSettings, VideoStageRole,
};
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
    let http = build_streaming_http_client(15, 30).unwrap();
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

#[tokio::test]
async fn asr_test_sends_real_audio_inference_to_qwen_asr() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-asr-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "message": {"content": ""}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::OpenAi,
        url: format!("{}/v1", server.uri()),
        model: "qwen3-asr-flash".into(),
        api_key: "sk-asr-key".into(),
    };
    let ms = test_stage_endpoint(&ep, true).await.unwrap();
    let _ = ms;
}

#[tokio::test]
async fn retries_on_http_429_rate_limit_and_recovers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_string(r#"{"error":{"code":"limit_requests","message":"Rate limit"}}"#),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "message": {"content": "{\"ok\": true}"}
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::OpenAi,
        url: format!("{}/v1", server.uri()),
        model: "gpt-4.1-mini".into(),
        api_key: "sk-test".into(),
    };
    let http = build_streaming_http_client(15, 30).unwrap();
    let out = call_stage_model(
        &http,
        &ep,
        "Return JSON.",
        &[UserPart::Text("Ping".into())],
        0.0,
        32,
        false,
    )
    .await
    .unwrap();
    assert!(out.text.contains("ok"));
}

#[test]
fn video_settings_deserializes_legacy_json_with_default_vlm_slot() {
    let legacy = r#"{"pipeline":"auto","transcript_policy":"prefer_any_subtitles","target_language":"","chapter_concurrency":2,"ffmpeg_path":"","yt_dlp_path":"","asr":{"protocol":"auto","url":"","model":"qwen3-asr-flash"},"stage1":{"protocol":"gemini","url":"https://example.com/v1beta","model":"gemini-3.8-flash"},"stage2":{"protocol":"auto","url":"","model":""}}"#;
    let parsed: VideoSettings = serde_json::from_str(legacy).unwrap();
    assert_eq!(parsed.stage1.model, "gemini-3.8-flash");
    assert!(parsed.vlm.url.is_empty());
    assert_eq!(VideoStageRole::Vlm.secret_key(), "video:vlm-key");
}

#[test]
fn bare_relay_url_defaults_to_openai_even_when_model_contains_gemini() {
    let bare_relay = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: "https://relay.example.com/".into(),
        model: "gemini-3.5-flash-lite".into(),
        api_key: "sk-relay".into(),
    };
    assert_eq!(bare_relay.resolved_protocol(), ResolvedProtocol::OpenAi);

    let google_official = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: "https://generativelanguage.googleapis.com".into(),
        model: "gemini-2.5-flash".into(),
        api_key: "AIzaSyTestKey123456789012345".into(),
    };
    assert_eq!(google_official.resolved_protocol(), ResolvedProtocol::Gemini);

    let google_openai_compat = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
        model: "gemini-2.5-flash".into(),
        api_key: "AIzaSyTestKey123456789012345".into(),
    };
    assert_eq!(
        google_openai_compat.resolved_protocol(),
        ResolvedProtocol::OpenAi
    );
}

#[tokio::test]
async fn streams_gemini_sse_chunks_skipping_thoughts_and_reassembling_json() {
    use wiremock::matchers::query_param;

    let server = MockServer::start().await;
    let sse_body = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"deep video reasoning...\",\"thought\":true}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"{\\\"article_\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"title\\\": \\\"Blurring at 20,000 FPS\\\"}\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-4-hydrogen:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "sk-station-secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream; charset=utf-8")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: format!("{}/v1beta", server.uri()),
        model: "gemini-4-hydrogen".into(),
        api_key: "sk-station-secret".into(),
    };
    let http = build_streaming_http_client(15, 30).unwrap();
    let out = call_stage_model(
        &http,
        &ep,
        "Return JSON.",
        &[UserPart::Text("Analyze clip".into())],
        0.15,
        16384,
        true,
    )
    .await
    .unwrap();
    let parsed = extract_json_object(&out.text).unwrap();
    assert_eq!(parsed["article_title"], "Blurring at 20,000 FPS");
}

#[tokio::test]
async fn retries_mid_body_truncation_and_surfaces_underlying_cause_on_exhaustion() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Case 1: First response truncates mid-chunk (`error decoding response body`), second succeeds.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_srv = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let n = hits_srv.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                if n == 0 {
                    let partial = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n10\r\ndata: {\"broken\"";
                    let _ = sock.write_all(partial).await;
                    drop(sock);
                } else {
                    let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"{\\\"recovered\\\": true}\"}]}}]}\n\n";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    let ep = StageEndpoint {
        protocol: ProtocolKind::Gemini,
        url: format!("http://{addr}/v1beta"),
        model: "gemini-4-hydrogen".into(),
        api_key: "sk-secret-recover".into(),
    };
    let http = build_streaming_http_client(15, 30).unwrap();
    let out = call_stage_model(
        &http,
        &ep,
        "",
        &[UserPart::Text("Test".into())],
        0.1,
        256,
        true,
    )
    .await
    .unwrap();
    let json = extract_json_object(&out.text).unwrap();
    assert_eq!(json["recovered"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // Case 2: Persistent mid-body truncation surfaces underlying cause chain and redacts key.
    let fail_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fail_addr = fail_listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = fail_listener.accept().await {
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let partial = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n20\r\n{\"incomplete\":";
                let _ = sock.write_all(partial).await;
                drop(sock);
            });
        }
    });

    let fail_ep = StageEndpoint {
        protocol: ProtocolKind::Gemini,
        url: format!("http://{fail_addr}/v1beta"),
        model: "gemini-4-hydrogen".into(),
        api_key: "sk-secret-exhausted".into(),
    };
    let err = call_gemini_generate_content_with_retries(
        &http,
        &fail_ep,
        "",
        &[UserPart::Text("Test".into())],
        0.1,
        256,
        true,
        0,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(!err.contains("sk-secret-exhausted"));
    // Must contain underlying transport cause (e.g., incomplete message / unexpected EOF),
    // not just the bare outer `error decoding response body for url (...)` wrapper.
    assert!(
        err.to_ascii_lowercase().contains("incomplete")
            || err.to_ascii_lowercase().contains("eof")
            || err.to_ascii_lowercase().contains("closed"),
        "expected detailed transport cause, got: {err}"
    );
}

#[tokio::test]
async fn streams_openai_sse_and_labels_mid_body_read_timeout() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let server = MockServer::start().await;
    let openai_sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"title\\\": \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"\\\"Box Blur\\\"}\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-openai-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_sse),
        )
        .expect(1)
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::Auto,
        url: format!("{}/v1", server.uri()),
        model: "claude-opus-5-5".into(),
        api_key: "sk-openai-stream".into(),
    };
    let http = build_streaming_http_client(10, 20).unwrap();
    let out = call_stage_model(
        &http,
        &ep,
        "Write JSON.",
        &[UserPart::Text("Chapter 1".into())],
        0.2,
        1024,
        true,
    )
    .await
    .unwrap();
    let parsed = extract_json_object(&out.text).unwrap();
    assert_eq!(parsed["title"], "Box Blur");

    // Verify mid-body stall surfaces `Request timed out` rather than bare `error decoding response body`.
    let stall_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stall_addr = stall_listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = stall_listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    let stall_ep = StageEndpoint {
        protocol: ProtocolKind::Gemini,
        url: format!("http://{stall_addr}/v1beta"),
        model: "gemini-4-hydrogen".into(),
        api_key: "sk-stall-secret".into(),
    };
    let short_http = build_streaming_http_client(1, 5).unwrap();
    let stall_err = call_gemini_generate_content_with_retries(
        &short_http,
        &stall_ep,
        "",
        &[UserPart::Text("Ping".into())],
        0.1,
        256,
        true,
        0,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(!stall_err.contains("sk-stall-secret"));
    assert!(
        stall_err.starts_with("Request timed out:"),
        "expected Request timed out prefix, got: {stall_err}"
    );
    assert!(
        stall_err.to_ascii_lowercase().contains("timed out"),
        "expected timed out details, got: {stall_err}"
    );
}


