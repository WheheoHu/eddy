//! Requests carried out on an engine in a temporary directory.

use std::sync::Arc;

use eddy_control::dto::*;
use eddy_control::{ControlError, DocFormat, Mark, ProgressTarget, Request, Scope, execute};
use eddy_core::{Engine, EngineOptions, MemoryPlatform};
use serde::de::DeserializeOwned;

fn open(dir: &std::path::Path) -> Engine {
    Engine::open(
        dir,
        Arc::new(MemoryPlatform::default()),
        EngineOptions::default(),
    )
    .unwrap()
}

async fn run<T: DeserializeOwned>(engine: &Engine, request: Request) -> T {
    let value = execute(engine, request, Arc::new(|_| {})).await.unwrap();
    serde_json::from_value(value).unwrap()
}

async fn refused(engine: &Engine, request: Request) -> ControlError {
    execute(engine, request, Arc::new(|_| {}))
        .await
        .unwrap_err()
}

fn list(scope: Scope) -> Request {
    Request::ItemsList {
        scope,
        unread: false,
        limit: 50,
        offset: 0,
    }
}

async fn add(engine: &Engine, title: &str, body: &str) -> AddedDto {
    run(
        engine,
        Request::AddText {
            title: Some(title.into()),
            body: body.into(),
            html: false,
            url: None,
        },
    )
    .await
}

#[tokio::test]
async fn text_goes_in_and_comes_back_out() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let added = add(
        &engine,
        "Tides",
        "The **sea** rises at $t_0$.\n\n- twice a day\n- higher at full moon\n",
    )
    .await;
    assert!(!added.existing);

    let page: ItemPage = run(&engine, list(Scope::Saved)).await;
    assert_eq!((page.total, page.items.len()), (1, 1));
    assert_eq!(page.items[0].id, added.id);
    assert_eq!(page.items[0].kind, "saved");
    assert_eq!(page.items[0].format, "article");
    assert_eq!(page.items[0].tags, None);

    let doc: DocDto = run(
        &engine,
        Request::Doc {
            id: added.id.clone(),
            format: DocFormat::Markdown,
            pages: None,
        },
    )
    .await;
    assert_eq!(
        doc.content.as_deref(),
        Some("# Tides\n\nThe **sea** rises at $t_0$.\n\n- twice a day\n- higher at full moon")
    );
    assert_eq!((doc.format.as_str(), doc.pages), ("markdown", None));

    let text: DocDto = run(
        &engine,
        Request::Doc {
            id: added.id.clone(),
            format: DocFormat::Text,
            pages: None,
        },
    )
    .await;
    assert!(text.content.unwrap().starts_with("The sea rises at $t_0$."));
    let json: DocDto = run(
        &engine,
        Request::Doc {
            id: added.id.clone(),
            format: DocFormat::Json,
            pages: None,
        },
    )
    .await;
    assert_eq!(json.content, None);
    assert_eq!(
        json.document.unwrap()["blocks"].as_array().unwrap().len(),
        3
    );

    // A PDF's pages are asked of something that has none.
    let no_pages = refused(
        &engine,
        Request::Doc {
            id: added.id.clone(),
            format: DocFormat::Markdown,
            pages: Some((1, 2)),
        },
    )
    .await;
    assert_eq!(no_pages.code, "invalid");

    let hits: SearchHits = run(
        &engine,
        Request::Search {
            query: "moon".into(),
            limit: 5,
            local: true,
        },
    )
    .await;
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].item.id, added.id);
}

#[tokio::test]
async fn items_are_marked_tagged_and_deleted_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let one = add(&engine, "One", "First text.").await.id;
    let two = add(&engine, "Two", "Second text.").await.id;

    // One wrong ID and nothing is changed.
    let missing = refused(
        &engine,
        Request::ItemsMark {
            ids: vec![one.clone(), "nope".into()],
            mark: Mark::Star,
        },
    )
    .await;
    assert_eq!(missing.code, "not_found");
    let starred: ItemPage = run(&engine, list(Scope::Starred)).await;
    assert_eq!(starred.total, 0);

    let marked: Count = run(
        &engine,
        Request::ItemsMark {
            ids: vec![one.clone(), two.clone()],
            mark: Mark::Star,
        },
    )
    .await;
    assert_eq!(marked.count, 2);
    let _: Count = run(
        &engine,
        Request::ItemsMark {
            ids: vec![two.clone()],
            mark: Mark::Unstar,
        },
    )
    .await;
    let _: Count = run(
        &engine,
        Request::ItemsMark {
            ids: vec![one.clone()],
            mark: Mark::Read,
        },
    )
    .await;

    let tagged: ItemTags = run(
        &engine,
        Request::ItemsTag {
            id: one.clone(),
            names: vec!["physics".into(), " sea ".into()],
            on: true,
        },
    )
    .await;
    assert_eq!(tagged.tags.len(), 2);
    let untagged: ItemTags = run(
        &engine,
        Request::ItemsTag {
            id: one.clone(),
            names: vec!["sea".into(), "never made".into()],
            on: false,
        },
    )
    .await;
    assert_eq!(untagged.tags, ["physics"]);
    let tags: Tags = run(&engine, Request::TagsList).await;
    assert_eq!(tags.tags.len(), 2, "taking a tag off an item keeps the tag");

    let got: Items = run(
        &engine,
        Request::ItemsGet {
            ids: vec![one.clone()],
        },
    )
    .await;
    let item = &got.items[0];
    assert!(item.starred && item.read && item.finished);
    assert_eq!(item.tags.as_deref(), Some(&["physics".to_string()][..]));

    let by_tag: ItemPage = run(&engine, list(Scope::Tag("physics".into()))).await;
    assert_eq!(by_tag.total, 1);
    let unread: ItemPage = run(
        &engine,
        Request::ItemsList {
            scope: Scope::Saved,
            unread: true,
            limit: 50,
            offset: 0,
        },
    )
    .await;
    assert_eq!(
        unread.items.iter().map(|i| &i.id).collect::<Vec<_>>(),
        [&two]
    );
    let no_tag = refused(&engine, list(Scope::Tag("chemistry".into()))).await;
    assert_eq!(no_tag.code, "not_found");

    let _: Removed = run(
        &engine,
        Request::TagsRename {
            name: "physics".into(),
            to: "Physics".into(),
        },
    )
    .await;
    let _: Removed = run(&engine, Request::TagsRemove { name: "sea".into() }).await;
    let tags: Tags = run(&engine, Request::TagsList).await;
    assert_eq!(tags.tags.len(), 1);
    assert_eq!(
        (tags.tags[0].name.as_str(), tags.tags[0].count),
        ("Physics", 1)
    );

    let deleted: Count = run(&engine, Request::ItemsDelete { ids: vec![two] }).await;
    assert_eq!(deleted.count, 1);
    let left: ItemPage = run(&engine, list(Scope::Saved)).await;
    assert_eq!(left.total, 1);
    let gone = refused(
        &engine,
        Request::ItemsDelete {
            ids: vec!["nope".into()],
        },
    )
    .await;
    assert_eq!(gone.code, "not_found");
}

#[tokio::test]
async fn reading_progress_is_set_by_block_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let body: String = (1..=12)
        .map(|n| format!("Paragraph {n} of a text long enough to be somewhere in.\n\n"))
        .collect();
    let id = add(&engine, "Long", &body).await.id;

    let before: ProgressDto = run(&engine, Request::ProgressGet { id: id.clone() }).await;
    assert_eq!(
        (before.progress, before.finished, before.block),
        (0.0, false, None)
    );

    let blocks: DocDto = run(
        &engine,
        Request::Doc {
            id: id.clone(),
            format: DocFormat::Blocks,
            pages: None,
        },
    )
    .await;
    let lines: Vec<String> = blocks.content.unwrap().lines().map(String::from).collect();
    assert_eq!(lines.len(), 12);
    assert!(
        lines[5].ends_with("[paragraph] Paragraph 6 of a text long enough to be somewhere in.")
    );
    let sixth = lines[5].split_whitespace().next().unwrap().to_string();

    let after: ProgressDto = run(
        &engine,
        Request::ProgressSet {
            id: id.clone(),
            to: ProgressTarget::Block {
                block: sixth.clone(),
                offset: 10,
            },
        },
    )
    .await;
    assert_eq!(after.block.as_deref(), Some(sixth.as_str()));
    assert_eq!(after.offset, Some(10));
    assert!(
        after.progress > 0.3 && after.progress < 0.7,
        "{}",
        after.progress
    );
    assert!(!after.finished && !after.from_other_device);
    let continuing: ItemPage = run(&engine, list(Scope::Continue)).await;
    assert_eq!(continuing.total, 1);

    let wrong = refused(
        &engine,
        Request::ProgressSet {
            id: id.clone(),
            to: ProgressTarget::Block {
                block: "nope".into(),
                offset: 0,
            },
        },
    )
    .await;
    assert_eq!(wrong.code, "invalid");
    let not_a_pdf = refused(
        &engine,
        Request::ProgressSet {
            id: id.clone(),
            to: ProgressTarget::Page { page: 1 },
        },
    )
    .await;
    assert_eq!(not_a_pdf.code, "invalid");

    let ended: ProgressDto = run(
        &engine,
        Request::ProgressSet {
            id,
            to: ProgressTarget::End,
        },
    )
    .await;
    assert!(ended.finished);
}

#[tokio::test]
async fn an_engine_with_no_account_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let engine = open(dir.path());
    let status: StatusDto = run(&engine, Request::Status).await;
    assert_eq!(status.mode, "unconfigured");
    assert_eq!((status.account, status.server), (None, None));
    let feeds: Feeds = run(&engine, Request::FeedsList).await;
    assert!(feeds.feeds.is_empty());
    let folders: Folders = run(&engine, Request::Folders).await;
    assert!(folders.folders.is_empty());
    let sync = refused(&engine, Request::Sync { history: false }).await;
    assert_eq!(sync.code, "not_configured");
    let hello: HelloDto = run(&engine, Request::Hello).await;
    assert_eq!(hello.control, eddy_control::CONTROL_VERSION);
    let summary: SummaryDto = {
        let id = add(&engine, "Short", "Nothing much.").await.id;
        run(
            &engine,
            Request::Summary {
                id,
                generate: false,
                force: false,
            },
        )
        .await
    };
    assert_eq!((summary.tldr, summary.cached), (None, false));
}

#[test]
fn requests_and_frames_have_the_documented_form() {
    use eddy_control::{Envelope, Frame, Progress};
    let line = r#"{"v":1,"core":"0.2.0","request":{"cmd":"items.list","args":{"scope":{"feed":"abc"},"unread":true,"limit":10,"offset":0}}}"#;
    let envelope: Envelope = serde_json::from_str(line).unwrap();
    assert_eq!(
        envelope.request,
        Request::ItemsList {
            scope: Scope::Feed("abc".into()),
            unread: true,
            limit: 10,
            offset: 0
        }
    );
    assert_eq!(serde_json::to_string(&envelope).unwrap(), line);
    assert_eq!(
        serde_json::to_string(&Request::Status).unwrap(),
        r#"{"cmd":"status"}"#
    );
    assert_eq!(
        serde_json::to_string(&Frame::Progress(Progress {
            phase: "downloading".into(),
            done: 3,
            total: Some(9)
        }))
        .unwrap(),
        r#"{"progress":{"phase":"downloading","done":3,"total":9}}"#
    );
    assert_eq!(
        serde_json::to_string(&Frame::Error(ControlError::new("not_found", "no item"))).unwrap(),
        r#"{"error":{"code":"not_found","message":"no item"}}"#
    );
    assert!(!Request::Status.mutates());
    assert!(Request::AddUrl { url: String::new() }.mutates());
    assert!(
        !Request::ItemsReadAll {
            scope: Scope::All,
            before: None,
            dry_run: true
        }
        .mutates()
    );
}
