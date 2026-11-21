//! Results as text for reading. `--json` prints the result itself instead.
//!
//! `scripts/test-server-smoke.sh` matches whole lines of `status`, `tags`
//! and `sign-out`; the tests below hold those lines.

use eddy_control::dto::*;
use eddy_control::{ControlError, Progress, Request};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn read<T: DeserializeOwned>(value: Value) -> Result<T, ControlError> {
    serde_json::from_value(value)
        .map_err(|e| ControlError::new("internal", format!("unexpected answer: {e}")))
}

/// The lines to print for the result of `request`; empty for nothing.
pub fn text(request: &Request, value: Value) -> Result<String, ControlError> {
    let mut out: Vec<String> = Vec::new();
    match request {
        Request::Hello => {
            let hello: HelloDto = read(value)?;
            out.push(format!("Eddy {} (process {})", hello.core, hello.pid));
        }
        Request::Status => out = status(&read(value)?),
        Request::Sync { .. } => {
            let sync: SyncDto = read(value)?;
            out.push(format!(
                "new items {}, state changes {}{}",
                sync.new_items,
                sync.state_changes,
                if sync.offline {
                    " (server offline: fetched from FreshRSS)"
                } else {
                    ""
                }
            ));
            if sync.history > 0 {
                out.push(format!("history: {} older articles", sync.history));
            }
        }
        Request::ItemsList { .. } => {
            let page: ItemPage = read(value)?;
            out.extend(page.items.iter().map(item_line));
        }
        Request::ItemsGet { .. } => {
            let items: Items = read(value)?;
            for (i, item) in items.items.iter().enumerate() {
                if i > 0 {
                    out.push(String::new());
                }
                out.extend(item_card(item));
            }
        }
        Request::ItemsMark { .. } | Request::ItemsDelete { .. } => {
            let count: Count = read(value)?;
            out.push(format!("{} done", count.count));
        }
        Request::ItemsReadAll { dry_run, .. } => {
            let count: Count = read(value)?;
            out.push(if *dry_run {
                format!("{} would be marked read", count.count)
            } else {
                format!("{} marked read", count.count)
            });
        }
        Request::ItemsFetch { .. } | Request::Doc { .. } => {
            let doc: DocDto = read(value)?;
            match (doc.content, doc.document) {
                (Some(content), _) => out.push(content),
                (None, Some(model)) => out.push(
                    serde_json::to_string_pretty(&model)
                        .map_err(|e| ControlError::new("internal", e.to_string()))?,
                ),
                (None, None) => {}
            }
        }
        Request::ItemsTag { .. } => {
            let tagged: ItemTags = read(value)?;
            out.push(format!("{}  {}", tagged.id, tagged.tags.join(", ")));
        }
        Request::AddUrl { .. } | Request::AddText { .. } => {
            let added: AddedDto = read(value)?;
            let verb = if added.existing {
                "already saved"
            } else {
                "saved"
            };
            out.push(format!("{verb} {} — {}", added.id, added.title));
        }
        Request::AddFile { .. } => {
            let added: AddedDto = read(value)?;
            let verb = if added.existing {
                "already imported"
            } else {
                "imported"
            };
            out.push(format!("{verb} {} — {}", added.id, added.title));
            if let Some(series) = &added.series_id {
                out.push(format!("  series {series}"));
            }
            for chapter in &added.chapters {
                out.push(format!(
                    "  {}  {:>6} chars  {}",
                    chapter.id, chapter.text_len, chapter.title
                ));
            }
        }
        Request::FeedsList => {
            let feeds: Feeds = read(value)?;
            for feed in feeds.feeds {
                out.push(format!(
                    "{}  {}  [{}]  unread {}",
                    feed.id,
                    feed.title,
                    feed.folder.as_deref().unwrap_or("-"),
                    feed.unread
                ));
            }
        }
        Request::FeedsAdd { .. } => {
            let added: FeedAdded = read(value)?;
            out.push(format!(
                "subscribed {} — {}{}",
                added.id,
                added.title,
                added
                    .folder
                    .map(|f| format!(" (in {f})"))
                    .unwrap_or_default()
            ));
        }
        Request::FeedsRemove { .. } => {
            let removed: Removed = read(value)?;
            out.push(format!("unsubscribed {}", removed.id));
        }
        Request::Folders => {
            let folders: Folders = read(value)?;
            for folder in folders.folders {
                out.push(format!(
                    "{}  feeds {}  unread {}",
                    folder.name, folder.feeds, folder.unread
                ));
            }
        }
        Request::Search { .. } => {
            let hits: SearchHits = read(value)?;
            for hit in hits.hits {
                out.push(format!(
                    "{}  {}  …{}…",
                    hit.item.id,
                    hit.item.title,
                    hit.snippet.replace('\n', " ")
                ));
            }
        }
        Request::ProgressGet { .. } | Request::ProgressSet { .. } => {
            let at: ProgressDto = read(value)?;
            out.push(format!("progress {:.0}%", at.progress * 100.0));
            out.push(format!("finished {}", at.finished));
            if let Some(block) = &at.block {
                out.push(format!("block {block} offset {}", at.offset.unwrap_or(0)));
            }
            if let Some(page) = at.page {
                out.push(format!("page {page}"));
            }
            if let Some(quote) = &at.quote {
                out.push(format!("at \"{}\"", quote.replace('\n', " ")));
            }
            if at.from_other_device {
                out.push("from another device".into());
            }
        }
        // Names alone, one a line: tags are named, not numbered.
        Request::TagsList => {
            let tags: Tags = read(value)?;
            out.extend(tags.tags.into_iter().map(|t| t.name));
        }
        Request::TagsAdd { .. } => {
            let tag: TagDto = read(value)?;
            out.push(format!("tag {}", tag.id));
        }
        Request::TagsRename { .. }
        | Request::TagsRemove { .. }
        | Request::SeriesAdd { .. }
        | Request::SeriesRemove { .. }
        | Request::SeriesRename { .. }
        | Request::SeriesDelete { .. } => {}
        Request::SeriesList => {
            let list: SeriesList = read(value)?;
            out.extend(list.series.iter().map(series_line));
        }
        Request::SeriesGet { .. } => {
            let detail: SeriesDetail = read(value)?;
            out.push(series_line(&detail.series));
            out.extend(
                detail
                    .chapters
                    .iter()
                    .map(|c| format!("  {}", item_line(c))),
            );
        }
        Request::SeriesDetect { .. } => {
            let found: SeriesFound = read(value)?;
            if !found.found {
                out.push("no table of contents found".into());
            } else {
                out.push(format!(
                    "{} — {} chapters",
                    found.title.unwrap_or_default(),
                    found.chapters.len()
                ));
                for chapter in found.chapters {
                    let indent = "  ".repeat(chapter.level as usize + 1);
                    out.push(format!("{indent}{}  {}", chapter.title, chapter.url));
                }
            }
        }
        Request::SeriesCreate { .. } => {
            let made: SeriesCreated = read(value)?;
            out.push(format!(
                "series {} — {} ({} chapters)",
                made.id, made.title, made.chapters
            ));
        }
        Request::Collection { .. } => {
            let collection: Collection = read(value)?;
            for entry in collection.entries {
                let chapters = match entry.chapters {
                    0 => String::new(),
                    n => format!("  {}/{n} chapters", entry.finished),
                };
                out.push(format!(
                    "{}  {:<10}  {:>3.0}%  {}{chapters}",
                    entry.id,
                    entry.kind,
                    entry.progress * 100.0,
                    entry.title
                ));
            }
        }
        Request::Summary { .. } => {
            let summary: SummaryDto = read(value)?;
            match summary.tldr {
                Some(tldr) => {
                    out.push(tldr);
                    out.extend(summary.points.iter().map(|p| format!("- {p}")));
                }
                None => out.push("no summary yet (--generate asks the model for one)".into()),
            }
        }
        Request::Translation { .. } => {
            let translation: TranslationDto = read(value)?;
            match translation.language {
                Some(language) => {
                    out.push(format!(
                        "{language}{}",
                        if translation.complete {
                            ""
                        } else {
                            " (not finished)"
                        }
                    ));
                    for (segment, text) in translation.segments {
                        out.push(format!("{segment}  {}", text.replace('\n', " ")));
                    }
                }
                None => out.push("not translated".into()),
            }
        }
        Request::ExportEpub { .. } => {
            let exported: Exported = read(value)?;
            out.push(format!("wrote {}", exported.path));
        }
    }
    Ok(out.join("\n"))
}

fn status(status: &StatusDto) -> Vec<String> {
    let mode = match status.mode.as_str() {
        "unconfigured" => "Unconfigured",
        "standalone" => "Standalone",
        "server" => "Server",
        other => other,
    };
    let mut out = vec![format!("mode {mode}")];
    if let Some(server) = &status.server {
        out.push(format!(
            "server {}",
            if server.online { "online" } else { "offline" }
        ));
        match (&server.version, server.cursor, &server.error) {
            (Some(version), Some(cursor), _) => {
                out.push(format!("server version {version}, cursor {cursor}"))
            }
            (_, _, Some(error)) => out.push(format!("server error: {error}")),
            _ => {}
        }
        out.push(format!("pending uploads {}", server.pending_uploads));
        out.push(format!("refused uploads {}", server.rejected_uploads));
        out.push(format!("offline refresh {}", server.offline_refresh));
    }
    if let Some(error) = &status.last_error {
        out.push(format!("last error {error}"));
    }
    out
}

/// `2026-11-21` in this machine's time zone.
fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| "----------".into())
}

fn marks(item: &ItemDto) -> String {
    [
        if item.read { '-' } else { 'u' },
        if item.starred { '*' } else { '-' },
        if item.finished { 'f' } else { '-' },
    ]
    .iter()
    .collect()
}

/// ID, date, state, title, feed.
fn item_line(item: &ItemDto) -> String {
    let feed = item
        .feed
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| format!("  — {f}"))
        .unwrap_or_default();
    format!(
        "{}  {}  {}  {}{feed}",
        item.id,
        day(item.published_at),
        marks(item),
        item.title
    )
}

fn item_card(item: &ItemDto) -> Vec<String> {
    let mut out = vec![
        format!("id        {}", item.id),
        format!("title     {}", item.title),
        format!("kind      {} ({})", item.kind, item.format),
    ];
    let mut line = |name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            out.push(format!("{name:<9} {value}"));
        }
    };
    line("url", item.url.clone());
    line("author", item.author.clone());
    line(
        "feed",
        item.feed
            .as_ref()
            .map(|f| format!("{f} ({})", item.feed_id.as_deref().unwrap_or("-"))),
    );
    line("series", item.series_id.clone());
    line("published", Some(day(item.published_at)));
    line(
        "state",
        Some(format!(
            "{}{}{}",
            if item.read { "read" } else { "unread" },
            if item.starred { ", starred" } else { "" },
            if item.finished { ", finished" } else { "" }
        )),
    );
    line("progress", Some(format!("{:.0}%", item.progress * 100.0)));
    line("length", Some(format!("{} characters", item.text_len)));
    line("language", item.lang.clone());
    line("tags", item.tags.as_ref().map(|t| t.join(", ")));
    line("excerpt", item.excerpt.clone());
    out
}

fn series_line(series: &SeriesDto) -> String {
    format!(
        "{}  {:>3.0}%  {}/{} chapters  {}",
        series.id,
        series.progress * 100.0,
        series.finished,
        series.chapters,
        series.title
    )
}

/// How far a long request is, for standard error.
pub fn progress(p: &Progress) -> String {
    let phase = match p.phase.as_str() {
        "fetching" => "fetching articles",
        other => other,
    };
    match (p.phase.as_str(), p.total) {
        ("history", _) => format!("history: {} older articles", p.done),
        (_, Some(total)) => format!("{phase}: {} of {total}", p.done),
        (_, None) => format!("{phase}: {}", p.done),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server(online: bool) -> Value {
        json!({
            "mode": "server", "account": null, "last_refresh": null, "last_error": null,
            "core": "0.2.0", "sync_protocol": 3,
            "server": {
                "online": online, "url": "http://127.0.0.1:1",
                "version": if online { json!("0.2.0") } else { Value::Null },
                "cursor": if online { json!(12) } else { Value::Null },
                "error": if online { Value::Null } else { json!("refused") },
                "pending_uploads": 1, "rejected_uploads": 0, "offline_refresh": false
            }
        })
    }

    // The lines `scripts/test-server-smoke.sh` looks for with `grep -qx`.
    #[test]
    fn status_keeps_the_lines_the_smoke_test_matches() {
        let online = text(&Request::Status, server(true)).unwrap();
        assert_eq!(
            online,
            "mode Server\nserver online\nserver version 0.2.0, cursor 12\npending uploads 1\nrefused uploads 0\noffline refresh false"
        );
        let offline = text(&Request::Status, server(false)).unwrap();
        assert!(offline.lines().any(|l| l == "server offline"));
        assert!(offline.lines().any(|l| l == "server error: refused"));
        let fresh = text(
            &Request::Status,
            json!({"mode": "unconfigured", "account": null, "last_refresh": null,
                   "last_error": null, "server": null, "core": "0.2.0", "sync_protocol": 3}),
        )
        .unwrap();
        assert_eq!(fresh, "mode Unconfigured");
    }

    #[test]
    fn tags_are_bare_names_and_nothing_when_there_are_none() {
        let two = json!({"tags": [{"id": "t1", "name": "smoke-a", "count": 0},
                                   {"id": "t2", "name": "Later", "count": 3}]});
        assert_eq!(text(&Request::TagsList, two).unwrap(), "smoke-a\nLater");
        assert_eq!(text(&Request::TagsList, json!({"tags": []})).unwrap(), "");
        let made = json!({"id": "t1", "name": "smoke-a", "count": 0});
        assert_eq!(
            text(
                &Request::TagsAdd {
                    name: "smoke-a".into()
                },
                made
            )
            .unwrap(),
            "tag t1"
        );
    }

    #[test]
    fn items_are_one_line_each_with_their_state() {
        let item = json!({
            "id": "i1", "kind": "feed", "format": "article", "title": "On Tides",
            "url": null, "author": null, "feed_id": "f1", "feed": "Sea Blog",
            "series_id": null, "chapter_index": null,
            "published_at": 0, "added_at": 0, "read": false, "starred": true,
            "finished": false, "progress": 0.0, "last_read_at": null, "text_len": 10,
            "lang": null, "excerpt": null
        });
        let page = json!({"items": [item], "total": 1, "limit": 50, "offset": 0});
        let line = text(
            &Request::ItemsList {
                scope: Default::default(),
                unread: false,
                limit: 50,
                offset: 0,
            },
            page,
        )
        .unwrap();
        let fields: Vec<&str> = line.split("  ").collect();
        assert_eq!(fields[0], "i1");
        assert_eq!(fields[1].len(), 10);
        assert_eq!(&fields[2..], ["u*-", "On Tides", "— Sea Blog"]);
    }

    #[test]
    fn progress_reads_like_before() {
        let line = |phase: &str, done, total| {
            progress(&Progress {
                phase: phase.into(),
                done,
                total,
            })
        };
        assert_eq!(line("uploading", 3, Some(9)), "uploading: 3 of 9");
        assert_eq!(line("fetching", 4, None), "fetching articles: 4");
        assert_eq!(line("history", 40, None), "history: 40 older articles");
    }
}
