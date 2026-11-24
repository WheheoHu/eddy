use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::client::{
    UserPart, build_streaming_http_client, call_gemini_generate_content_with_retries,
    call_stage_model, extract_json_object, test_stage_endpoint,
};
use crate::epub_builder::{build_epub_bytes, format_takeaway_html};
use crate::media::parse_vtt_or_srt;
use crate::models::{ArticleBlock, ArticleDocument, ArticleSection, Stage1Plan, VideoProbe};
use crate::pipeline::{
    build_fallback_overview, merge_chunk_chapters_and_meta, sample_takeaways_proportionally,
    synthesize_chunked_overview,
};
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
    let joined = segs
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
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
                brief: "介绍 Cutout 渲染管线。".into(),
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
                brief: "深入解析辐射级联算法。".into(),
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
    assert_eq!(
        google_official.resolved_protocol(),
        ResolvedProtocol::Gemini
    );

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
        .and(path(
            "/v1beta/models/gemini-4-hydrogen:streamGenerateContent",
        ))
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

#[tokio::test]
async fn synthesizes_chunked_overview_via_stage2_endpoint() {
    use wiremock::matchers::body_string_contains;

    let parts = vec![
        (
            0usize,
            0u64,
            900_000u64,
            serde_json::json!({
                "article_title": "Smolder 体积渲染系统（第一部分）",
                "subtitle": "立项背景与核心需求",
                "executive_summary": "本部分演讲介绍了 Smolder 在 Glacier 引擎中的立项背景与核心需求。",
                "key_takeaways": ["立项背景：打造一等公民体积渲染", "缩放目标：覆盖 RTX 5090 到 Switch 2"],
                "chapters": [
                    {"chapter_index": 1, "title": "立项背景", "start_time": "00:00", "end_time": "08:00", "summary_goal": "Explain goals", "slides": []},
                    {"chapter_index": 2, "title": "光照集成挑战", "start_time": "08:00", "end_time": "15:00", "summary_goal": "Explain lighting", "slides": []}
                ]
            }),
        ),
        (
            1usize,
            900_000u64,
            1_800_000u64,
            serde_json::json!({
                "article_title": "Smolder 原型探索",
                "subtitle": "两次技术尝试",
                "executive_summary": "本部分演讲回顾了 GPU 粒子方案失败与 Raymarching + MBOIT 原型验证。",
                "key_takeaways": ["原型验证：回归光线步进与 MBOIT", "离线烘焙：规避 NanoVDB 寄存器压力"],
                "chapters": [
                    {"chapter_index": 1, "title": "原型演进", "start_time": "00:00", "end_time": "15:00", "summary_goal": "Explain prototypes", "slides": []}
                ]
            }),
        ),
        (
            2usize,
            1_800_000u64,
            2_700_000u64,
            serde_json::json!({
                "article_title": "Smolder 数据格式",
                "subtitle": "体素砖块设计",
                "executive_summary": "本部分演讲深入剖析了 4x4x4 体素砖块格式与 CuteSort 排序管线。",
                "key_takeaways": ["体素格式：4 bits/voxel 压缩与 5x5x5 边界复制", "重叠排序：波前剔除与 CuteSort"],
                "chapters": [
                    {"chapter_index": 1, "title": "运行时体积格式", "start_time": "00:00", "end_time": "15:00", "summary_goal": "Explain brick format", "slides": []}
                ]
            }),
        ),
        (
            3usize,
            2_700_000u64,
            3_600_000u64,
            serde_json::json!({
                "article_title": "Smolder 阴影与优化",
                "subtitle": "异步阴影与自适应采样",
                "executive_summary": "本部分演讲介绍了异步阴影计算、自适应采样组与 Xbox Series S 内存优化。",
                "key_takeaways": ["自适应采样：重度烟雾场景节省 50% 性能", "内存削减：半分辨率 Mip 降低 80%-85% 占用"],
                "chapters": [
                    {"chapter_index": 1, "title": "阴影与全平台缩放", "start_time": "00:00", "end_time": "15:00", "summary_goal": "Explain shadows and TAA", "slides": []}
                ]
            }),
        ),
    ];

    let (part_metas, merged_chapters) = merge_chunk_chapters_and_meta(&parts, 3_600_000);
    assert_eq!(part_metas.len(), 4);
    assert_eq!(merged_chapters.len(), 5);
    assert_eq!(merged_chapters[4]["chapter_index"], 5);
    assert_eq!(merged_chapters[4]["start_ms"], 2_700_000);

    let server = MockServer::start().await;
    let synth_json = serde_json::json!({
        "article_title": "《007 First Light》实时体积渲染系统 Smolder 架构解析",
        "subtitle": "IO Interactive 如何在 Glacier 引擎中打造全平台可扩展的体积特效管线",
        "executive_summary": "IO Interactive 在 SIGGRAPH 演讲中系统拆解了 Glacier 引擎的新一代体积渲染系统 Smolder。团队放弃 GPU 粒子方案，围绕自定义 4x4x4 压缩体素砖块、MBOIT 阴影与自适应采样构建统一管线，最终在全平台稳定运行超过 450 个动态体积特效。",
        "key_takeaways": [
            "管线定型：放弃软件光栅化粒子，转向离线烘焙加 Raymarching 与 MBOIT 方案。",
            "定制格式：4 bits/voxel 块压缩与 5x5x5 边界复制兼顾采样效率与低寄存器压力。",
            "异步阴影：利用 Async Compute 与动态阴影剔除实现近乎零成本的全光源体积阴影。",
            "性能缩放：自适应采样组节省高达 50% 重度烟雾开销，半分辨率 Mip 降低 80%-85% 显存占用。"
        ],
        "chapter_briefs": [
            {"chapter_index": 1, "brief": "介绍 Smolder 在 Glacier 引擎中的立项动机与跨平台目标。"},
            {"chapter_index": 2, "brief": "拆解太阳阴影、局部光源与全局光照的体积集成挑战。"},
            {"chapter_index": 3, "brief": "复盘两次早期原型实验以及转向离线烘焙模拟的权衡。"},
            {"chapter_index": 4, "brief": "详解自定义体素砖块格式、空域跳过与多体积排序算法。"},
            {"chapter_index": 5, "brief": "阐述异步阴影、蓝噪声自适应采样与主机内存优化实践。"}
        ]
    });

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("Part 1 of 4"))
        .and(body_string_contains("Part 4 of 4"))
        .and(body_string_contains("Chapter 5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{
                "message": {
                    "content": synth_json.to_string()
                }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let ep = StageEndpoint {
        protocol: ProtocolKind::OpenAi,
        url: format!("{}/v1", server.uri()),
        model: "claude-opus-5-5".into(),
        api_key: "sk-synth-test".into(),
    };
    let probe = VideoProbe {
        video_id: "MFgptB5tpxk".into(),
        title: "Smolder: Real-Time Volumetric Rendering".into(),
        duration_ms: 3_600_000,
        ..Default::default()
    };
    let http = build_streaming_http_client(10, 20).unwrap();
    let overview = synthesize_chunked_overview(
        &http,
        &ep,
        &probe,
        "Simplified Chinese",
        &part_metas,
        &merged_chapters,
    )
    .await
    .unwrap();

    assert_eq!(
        overview.article_title,
        "《007 First Light》实时体积渲染系统 Smolder 架构解析"
    );
    assert!(!overview.executive_summary.contains("本部分演讲"));
    assert_eq!(overview.key_takeaways.len(), 4);
    assert_eq!(overview.chapter_briefs.len(), 5);
    assert_eq!(
        overview.chapter_briefs.get(&5).map(String::as_str),
        Some("阐述异步阴影、蓝噪声自适应采样与主机内存优化实践。")
    );
}

#[test]
fn chunked_overview_fallback_samples_all_parts_and_splits_paragraphs() {
    let parts = vec![
        (
            0usize,
            0u64,
            900_000u64,
            serde_json::json!({
                "article_title": "Talk Part 1",
                "subtitle": "Framing",
                "executive_summary": "First part intro sentence. Second sentence here. Third verbose sentence that should be trimmed.",
                "key_takeaways": ["P1-A", "P1-B", "P1-C", "P1-D"],
                "chapters": []
            }),
        ),
        (
            1usize,
            900_000u64,
            1_800_000u64,
            serde_json::json!({
                "article_title": "Talk Part 2",
                "subtitle": "",
                "executive_summary": "Second part intro. Second part detail.",
                "key_takeaways": ["P2-A", "P1-A", "P2-B", "P2-C"],
                "chapters": []
            }),
        ),
        (
            2usize,
            1_800_000u64,
            2_700_000u64,
            serde_json::json!({
                "article_title": "Talk Part 3",
                "subtitle": "",
                "executive_summary": "Third part intro.",
                "key_takeaways": ["P3-A", "P3-B"],
                "chapters": []
            }),
        ),
        (
            3usize,
            2_700_000u64,
            3_600_000u64,
            serde_json::json!({
                "article_title": "Talk Part 4",
                "subtitle": "",
                "executive_summary": "Fourth part intro.",
                "key_takeaways": ["P4-A", "P4-B"],
                "chapters": []
            }),
        ),
    ];

    let (metas, _) = merge_chunk_chapters_and_meta(&parts, 3_600_000);
    let sampled = sample_takeaways_proportionally(&metas, 6);
    assert_eq!(sampled.len(), 6);
    // Round 0 must cover all 4 parts (fixing part 3-4 starvation), and duplicate "P1-A" is skipped.
    assert_eq!(sampled[0], "P1-A");
    assert_eq!(sampled[1], "P2-A");
    assert_eq!(sampled[2], "P3-A");
    assert_eq!(sampled[3], "P4-A");
    assert_eq!(sampled[4], "P1-B");
    assert_eq!(sampled[5], "P3-B");

    let fallback = build_fallback_overview("Fallback Probe Title", &metas);
    assert_eq!(fallback.article_title, "Talk Part 1");
    assert_eq!(fallback.subtitle, "Framing");
    // Paragraphs are split with blank lines rather than glued with spaces.
    assert_eq!(fallback.executive_summary.split("\n\n").count(), 4);
    assert!(
        !fallback
            .executive_summary
            .contains("Third verbose sentence")
    );
}

#[test]
fn format_takeaway_bolds_lead_in_topic_and_strips_markdown_stars() {
    assert_eq!(
        format_takeaway_html("内存带宽优化：通过片上缓存减少 60% 开销。"),
        "<strong>内存带宽优化：</strong>通过片上缓存减少 60% 开销。"
    );
    assert_eq!(
        format_takeaway_html("Async Shadows: Zero-cost volume shadows via Async Compute."),
        "<strong>Async Shadows: </strong>Zero-cost volume shadows via Async Compute."
    );
    assert_eq!(
        format_takeaway_html("**自适应采样**：重度烟雾场景节省 50% 性能。"),
        "<strong>自适应采样：</strong>重度烟雾场景节省 50% 性能。"
    );
    assert_eq!(
        format_takeaway_html("**Custom Format**: 4 bits/voxel with 5x5x5 border."),
        "<strong>Custom Format: </strong>4 bits/voxel with 5x5x5 border."
    );
    // URLs and timestamps must not be split into bold labels.
    assert_eq!(
        format_takeaway_html("See https://example.com for benchmark details."),
        "See https://example.com for benchmark details."
    );
    assert_eq!(
        format_takeaway_html("12:30: live profiling demo shows 2ms frame time."),
        "12:30: live profiling demo shows 2ms frame time."
    );
    assert_eq!(
        format_takeaway_html("Plain insight with no colon."),
        "Plain insight with no colon."
    );
}

#[test]
fn overview_epub_round_trips_through_eddy_epub_and_eddy_doc() {
    use eddy_doc::{ConvertOptions, html_to_document};
    use eddy_epub::Epub;
    use eddy_model::{BlockKind, style};
    use std::io::Cursor;

    // 1. Multi-chapter Chinese book
    let zh_doc = ArticleDocument {
        video_id: "MFgptB5tpxk".into(),
        title: "Smolder 实时体积渲染系统".into(),
        subtitle: "Glacier 引擎跨平台体积特效实践".into(),
        author: Some("Alexander Mueller".into()),
        source_url: Some("https://www.youtube.com/watch?v=MFgptB5tpxk".into()),
        duration_ms: 3_600_000,
        executive_summary: "第一段简明导读。\n\n第二段补充核心成果。".into(),
        key_takeaways: vec![
            "定制体素格式：4 bits/voxel 块压缩大幅降低显存占用。".into(),
            "**自适应采样**：近相机重度烟雾节省 50% 性能。".into(),
        ],
        sections: vec![
            ArticleSection {
                index: 1,
                title: "立项背景与核心需求".into(),
                start_ms: 0,
                end_ms: 1_800_000,
                brief: "介绍 Smolder 的跨平台目标与光照挑战。".into(),
                blocks: vec![ArticleBlock {
                    kind: "paragraph".into(),
                    text: "第一章正文内容。".into(),
                    ..Default::default()
                }],
            },
            ArticleSection {
                index: 2,
                title: "底层格式与阴影优化".into(),
                start_ms: 1_800_000,
                end_ms: 3_600_000,
                brief: String::new(),
                blocks: vec![ArticleBlock {
                    kind: "paragraph".into(),
                    text: "第二章正文内容。".into(),
                    ..Default::default()
                }],
            },
        ],
    };

    let (epub_bytes, ch_count) = build_epub_bytes(&zh_doc).unwrap();
    assert_eq!(ch_count, 3);
    let mut epub = Epub::new(Cursor::new(epub_bytes)).unwrap();
    let book = epub.book().unwrap();
    assert_eq!(book.chapters.len(), 3);
    assert_eq!(book.chapters[0].title.as_deref(), Some("导读与核心要点"));

    let ov_bytes = epub.read(&book.chapters[0].parts[0]).unwrap();
    let ov_html = String::from_utf8(ov_bytes).unwrap();
    let parsed = html_to_document(
        &ov_html,
        &ConvertOptions {
            base_url: Some("epub://testkey/OEBPS/chapter_00.xhtml"),
            title: Some("导读与核心要点"),
        },
    );

    let headings: Vec<String> = parsed
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Heading { level: 3, runs } => {
                Some(runs.iter().map(|r| r.text.as_str()).collect::<String>())
            }
            _ => None,
        })
        .collect();
    assert_eq!(headings, vec!["核心概览", "核心要点", "章节导览"]);

    let quoted_paras: Vec<String> = parsed
        .blocks
        .iter()
        .filter(|b| b.quote_depth == 1)
        .filter_map(|b| match &b.kind {
            BlockKind::Paragraph { runs } => {
                Some(runs.iter().map(|r| r.text.as_str()).collect::<String>())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        quoted_paras,
        vec!["第一段简明导读。", "第二段补充核心成果。"]
    );

    let unordered_items: Vec<&[eddy_model::Run]> = parsed
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::ListItem {
                ordered: false,
                runs,
                ..
            } => Some(runs.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(unordered_items.len(), 2);
    assert!(unordered_items[0][0].style & style::BOLD != 0);
    assert_eq!(unordered_items[0][0].text, "定制体素格式：");
    assert!(unordered_items[1][0].style & style::BOLD != 0);
    assert_eq!(unordered_items[1][0].text, "自适应采样：");
    assert!(!ov_html.contains("**"));

    let ordered_items: Vec<&[eddy_model::Run]> = parsed
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::ListItem {
                ordered: true,
                runs,
                ..
            } => Some(runs.as_slice()),
            _ => None,
        })
        .collect();
    assert_eq!(ordered_items.len(), 2);
    assert_eq!(
        ordered_items[0][0].link.as_deref(),
        Some("epub://testkey/OEBPS/chapter_01.xhtml")
    );
    let ch1_text: String = ordered_items[0].iter().map(|r| r.text.as_str()).collect();
    assert!(ch1_text.contains("介绍 Smolder 的跨平台目标与光照挑战。"));
    let ch2_text: String = ordered_items[1].iter().map(|r| r.text.as_str()).collect();
    assert!(!ch2_text.contains("："));

    // 2. Single-chapter English article omits Chapter Guide
    let en_doc = ArticleDocument {
        video_id: "short_en".into(),
        title: "Compact Raymarching".into(),
        subtitle: "Fast volumes on mobile GPUs".into(),
        author: None,
        source_url: None,
        duration_ms: 480_000,
        executive_summary: "A concise two-sentence summary. It runs in under 1ms.".into(),
        key_takeaways: vec!["Memory: Fits inside tile memory.".into()],
        sections: vec![ArticleSection {
            index: 1,
            title: "Architecture".into(),
            start_ms: 0,
            end_ms: 480_000,
            brief: "Covers tile layout.".into(),
            blocks: vec![ArticleBlock {
                kind: "paragraph".into(),
                text: "Body text.".into(),
                ..Default::default()
            }],
        }],
    };
    let (en_bytes, en_count) = build_epub_bytes(&en_doc).unwrap();
    assert_eq!(en_count, 1);
    let mut en_epub = Epub::new(Cursor::new(en_bytes)).unwrap();
    let en_book = en_epub.book().unwrap();
    let en_html = String::from_utf8(en_epub.read(&en_book.chapters[0].parts[0]).unwrap()).unwrap();
    assert!(en_html.contains("<h3>Overview</h3>"));
    assert!(en_html.contains("<h3>Key Takeaways</h3>"));
    assert!(!en_html.contains("Chapter Guide"));
    assert!(en_html.contains("Duration 08:00"));
}

#[test]
fn deserializes_legacy_checkpoints_without_brief_field() {
    let legacy_plan = serde_json::json!({
        "article_title": "Legacy Book",
        "subtitle": "Sub",
        "executive_summary": "Summary",
        "key_takeaways": ["Point 1"],
        "chapters": [{
            "chapter_index": 1,
            "title": "Ch 1",
            "start_ms": 0,
            "end_ms": 60000,
            "summary_goal": "Goal",
            "key_terms": [],
            "selected_slides": []
        }]
    });
    let plan: Stage1Plan = serde_json::from_value(legacy_plan).unwrap();
    assert_eq!(plan.chapters[0].brief, "");

    let legacy_sec = serde_json::json!({
        "index": 1,
        "title": "Ch 1",
        "start_ms": 0,
        "end_ms": 60000,
        "blocks": []
    });
    let sec: ArticleSection = serde_json::from_value(legacy_sec).unwrap();
    assert_eq!(sec.brief, "");
}
