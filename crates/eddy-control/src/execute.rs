//! Every request, carried out on an engine. The command line runs this in
//! its own process on a data directory of its own, and the app runs it for
//! the command line on the user's data; there is one implementation.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use eddy_core::{
    CORE_VERSION, Engine, ExportContentMode, ExportRequest, ExportScope, ItemFilter, ItemQuery,
    ItemRow, Mode, SYNC_PROTOCOL_VERSION, SyncPhase, TextFormat, core_version,
};
use eddy_model::{BlockKind, Document, MarkdownOptions, SourceKind, ids, now_ms};
use serde::Serialize;
use serde_json::Value;

use crate::dto::*;
use crate::protocol::{
    CONTROL_VERSION, ControlError, DocFormat, Mark, Progress, ProgressTarget, Request, Scope,
};

/// Called as a long request moves.
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

type Outcome<T> = Result<T, ControlError>;

/// More chapters than any series has.
const ALL_CHAPTERS: u32 = 100_000;

fn done(value: impl Serialize) -> Outcome<Value> {
    Ok(serde_json::to_value(value)?)
}

pub async fn execute(engine: &Engine, request: Request, progress: ProgressFn) -> Outcome<Value> {
    match request {
        Request::Hello => done(HelloDto {
            control: CONTROL_VERSION,
            core: core_version(),
            pid: std::process::id(),
        }),
        Request::Status => done(status(engine).await?),
        Request::Sync { history } => done(sync(engine, history, progress).await?),

        Request::ItemsList {
            scope,
            unread,
            limit,
            offset,
        } => {
            let query = ItemQuery {
                filter: filter(engine, &scope)?,
                unread_only: unread,
                keep: Vec::new(),
                limit,
                offset,
            };
            done(ItemPage {
                items: engine.items(&query)?.iter().map(ItemDto::from).collect(),
                total: engine.item_count(&query)?,
                limit,
                offset,
            })
        }
        Request::ItemsGet { ids } => {
            let mut items = Vec::with_capacity(ids.len());
            for id in &ids {
                let row = item(engine, id)?;
                let tags = engine.item_tags(id)?.tags;
                items.push(ItemDto {
                    tags: Some(tags.into_iter().map(|t| t.name).collect()),
                    ..ItemDto::from(&row)
                });
            }
            done(Items { items })
        }
        Request::ItemsMark { ids, mark } => {
            // All or nothing: a mistyped ID changes no other item.
            for id in &ids {
                item(engine, id)?;
            }
            match mark {
                Mark::Read => engine.mark_read(&ids)?,
                Mark::Unread => engine.mark_unread(&ids)?,
                Mark::Star | Mark::Unstar => {
                    for id in &ids {
                        engine.set_starred(id, mark == Mark::Star).await?;
                    }
                }
                Mark::Finish | Mark::Unfinish => {
                    for id in &ids {
                        engine.set_finished(id, mark == Mark::Finish)?;
                    }
                }
            }
            done(Count {
                count: ids.len() as u32,
            })
        }
        Request::ItemsReadAll {
            scope,
            before,
            dry_run,
        } => {
            let filter = filter(engine, &scope)?;
            let until = before.unwrap_or_else(now_ms);
            let count = if dry_run {
                engine.unread_feed_count(&filter, until)?
            } else {
                engine.mark_all_read(&filter, until)?
            };
            done(Count { count })
        }
        Request::ItemsDelete { ids } => {
            for id in &ids {
                if engine.item(id)?.is_none() && engine.series_info(id)?.is_none() {
                    return Err(ControlError::not_found("item or series", id));
                }
            }
            for id in &ids {
                engine.delete_entry(id).await?;
            }
            done(Count {
                count: ids.len() as u32,
            })
        }
        Request::ItemsFetch { id } => {
            let row = item(engine, &id)?;
            let loaded = engine.fetch_full_text(&id).await?;
            done(doc(&row, &loaded.document, DocFormat::Summary, None)?)
        }
        Request::ItemsTag { id, names, on } => {
            item(engine, &id)?;
            for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
                let existing = engine.tags()?.into_iter().find(|t| t.name == name);
                let tag = match (existing, on) {
                    (Some(tag), _) => tag.id,
                    (None, true) => engine.create_tag(name)?,
                    (None, false) => continue,
                };
                engine.set_item_tag(&id, &tag, on)?;
            }
            let tags = engine.item_tags(&id)?.tags;
            done(ItemTags {
                id,
                tags: tags.into_iter().map(|t| t.name).collect(),
            })
        }

        Request::Doc { id, format, pages } => {
            let row = item(engine, &id)?;
            let document = document(engine, &row).await?;
            done(doc(&row, &document, format, pages)?)
        }

        Request::AddUrl { url } => {
            let existing = engine.saved_item_for_url(&url)?.is_some();
            let row = engine.save_url(&url).await?;
            done(AddedDto {
                id: row.item.id,
                title: row.item.title,
                existing,
                series_id: None,
                chapters: Vec::new(),
            })
        }
        Request::AddFile { path } => {
            let imported = engine.import_file(std::path::Path::new(&path)).await?;
            let chapters = match &imported.series_id {
                Some(series) => chapters(engine, series)?
                    .iter()
                    .map(|r| ChapterDto {
                        id: r.item.id.clone(),
                        title: r.item.title.clone(),
                        text_len: r.item.text_len,
                    })
                    .collect(),
                None => Vec::new(),
            };
            done(AddedDto {
                id: imported.item_id,
                title: imported.title,
                existing: imported.existing,
                series_id: imported.series_id,
                chapters,
            })
        }
        Request::AddText {
            title,
            body,
            html,
            url,
        } => {
            let format = if html {
                TextFormat::Html
            } else {
                TextFormat::Markdown
            };
            let row = engine
                .save_text(title.as_deref(), &body, format, url.as_deref())
                .await?;
            done(AddedDto {
                id: row.item.id,
                title: row.item.title,
                existing: false,
                series_id: None,
                chapters: Vec::new(),
            })
        }

        Request::FeedsList => done(Feeds {
            feeds: feeds(engine)?,
        }),
        Request::FeedsAdd { url, folder } => {
            let folder = folder.as_deref().map(str::trim).filter(|f| !f.is_empty());
            let added = engine.add_feed(&url, folder).await?;
            done(FeedAdded {
                id: added.source_id,
                title: added.title,
                folder: added.folder,
            })
        }
        Request::FeedsRemove { id } => {
            if !feeds(engine)?.iter().any(|f| f.id == id) {
                return Err(ControlError::not_found("feed", &id));
            }
            engine.remove_feed(&id).await?;
            done(Removed { id })
        }
        Request::Folders => {
            let mut folders: BTreeMap<String, FolderDto> = BTreeMap::new();
            for feed in feeds(engine)? {
                let Some(name) = feed.folder else { continue };
                let folder = folders.entry(name.clone()).or_insert(FolderDto {
                    name,
                    feeds: 0,
                    unread: 0,
                });
                folder.feeds += 1;
                folder.unread += feed.unread;
            }
            done(Folders {
                folders: folders.into_values().collect(),
            })
        }

        Request::Search {
            query,
            limit,
            local,
        } => {
            let found = if local {
                engine.search(&query, limit)?
            } else {
                engine.search_all(&query, limit).await?
            };
            done(SearchHits {
                hits: found
                    .iter()
                    .map(|hit| SearchHitDto {
                        item: ItemDto::from(&hit.item),
                        snippet: hit.snippet.clone(),
                    })
                    .collect(),
            })
        }

        Request::ProgressGet { id } => done(reading(engine, &id)?),
        Request::ProgressSet { id, to } => {
            let row = item(engine, &id)?;
            match to {
                ProgressTarget::Block { block, offset } => {
                    let document = document(engine, &row).await?;
                    if document.block(&block).is_none() {
                        return Err(ControlError::invalid(format!(
                            "the item has no block {block} (`doc {id} --format blocks` lists them)"
                        )));
                    }
                    engine.record_progress(&id, &block, offset, false, true)?;
                }
                ProgressTarget::Page { page } => {
                    let count = document(engine, &row).await?.pages.len() as u32;
                    if page == 0 || page > count {
                        return Err(ControlError::invalid(match count {
                            0 => "the item has no pages: it is not a PDF".to_string(),
                            n => format!("page {page} is outside the PDF's {n} pages"),
                        }));
                    }
                    engine.record_page_progress(&id, page - 1, count, true)?;
                }
                ProgressTarget::End => engine.set_finished(&id, true)?,
            }
            done(reading(engine, &id)?)
        }

        Request::TagsList => done(Tags {
            tags: engine
                .tags()?
                .into_iter()
                .map(|t| TagDto {
                    id: t.id,
                    name: t.name,
                    count: t.count,
                })
                .collect(),
        }),
        Request::TagsAdd { name } => {
            let id = engine.create_tag(&name)?;
            done(TagDto {
                id,
                name: name.trim().to_string(),
                count: 0,
            })
        }
        Request::TagsRename { name, to } => {
            let id = tag(engine, &name)?;
            engine.rename_tag(&id, &to)?;
            done(Removed { id })
        }
        Request::TagsRemove { name } => {
            let id = tag(engine, &name)?;
            engine.delete_tag(&id)?;
            done(Removed { id })
        }

        Request::SeriesList => done(SeriesList {
            series: engine.series_list()?.iter().map(SeriesDto::from).collect(),
        }),
        Request::SeriesGet { id } => {
            let info = series(engine, &id)?;
            done(SeriesDetail {
                series: SeriesDto::from(&info),
                chapters: chapters(engine, &id)?.iter().map(ItemDto::from).collect(),
            })
        }
        Request::SeriesDetect { url } => {
            let proposal = engine.detect_series_at(&url).await?;
            done(match proposal {
                Some(p) => SeriesFound {
                    found: true,
                    title: Some(p.title),
                    url: Some(p.toc_url),
                    chapters: p
                        .chapters
                        .into_iter()
                        .map(|c| ChapterLinkDto {
                            title: c.title,
                            url: c.url,
                            level: c.level,
                        })
                        .collect(),
                },
                None => SeriesFound {
                    found: false,
                    title: None,
                    url: None,
                    chapters: Vec::new(),
                },
            })
        }
        Request::SeriesCreate { url, title } => {
            let Some(proposal) = engine.detect_series_at(&url).await? else {
                return Err(ControlError::new(
                    "not_found",
                    format!("no table of contents found at {url}"),
                ));
            };
            let title = title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(&proposal.title)
                .to_string();
            let id = engine.create_series(&title, Some(&proposal.toc_url), &proposal.chapters)?;
            done(SeriesCreated {
                id,
                title,
                chapters: proposal.chapters.len() as u32,
            })
        }
        Request::SeriesAdd {
            id,
            item: item_id,
            index,
        } => {
            let info = series(engine, &id)?;
            item(engine, &item_id)?;
            engine.add_to_series(&id, &item_id, index.unwrap_or(info.chapters))?;
            done(Removed { id })
        }
        Request::SeriesRemove { item: item_id } => {
            item(engine, &item_id)?;
            engine.remove_from_series(&item_id)?;
            done(Removed { id: item_id })
        }
        Request::SeriesRename { id, title } => {
            series(engine, &id)?;
            engine.rename_series(&id, &title)?;
            done(Removed { id })
        }
        Request::SeriesDelete { id } => {
            series(engine, &id)?;
            engine.delete_entry(&id).await?;
            done(Removed { id })
        }
        Request::Collection { kind } => {
            let entries: Vec<CollectionEntryDto> = engine
                .collection()?
                .iter()
                .map(CollectionEntryDto::from)
                .filter(|e| kind.as_deref().is_none_or(|k| k == e.kind))
                .collect();
            done(Collection { entries })
        }

        Request::Summary {
            id,
            generate,
            force,
        } => {
            item(engine, &id)?;
            let cached = engine.cached_summary(&id)?;
            let (summary, cached) = match cached {
                Some(summary) if !(generate && force) => (Some(summary), true),
                _ if generate => {
                    let made = engine.summarize(&id, force).await?;
                    (Some(made.summary), made.cached)
                }
                _ => (None, false),
            };
            done(SummaryDto {
                id,
                tldr: summary.as_ref().map(|s| s.tldr.clone()),
                points: summary.map(|s| s.points).unwrap_or_default(),
                cached,
            })
        }
        Request::Translation { id } => {
            item(engine, &id)?;
            let cached = engine.cached_translation(&id)?;
            done(TranslationDto {
                id,
                language: cached.as_ref().map(|t| t.target_language.clone()),
                complete: cached.as_ref().is_some_and(|t| t.complete),
                segments: cached
                    .map(|t| t.segments.into_iter().collect())
                    .unwrap_or_default(),
            })
        }
        Request::ExportEpub { id, path } => {
            let options = engine.export_options(&id, false)?;
            if !options.exportable {
                return Err(ControlError::invalid(format!(
                    "{id} cannot be exported as an EPUB"
                )));
            }
            let report = progress.clone();
            engine
                .export_epub(
                    ExportRequest {
                        job_id: ids::new_local_id(),
                        target_id: id,
                        destination_path: path.clone(),
                        scope: ExportScope::Whole,
                        mode: ExportContentMode::Original,
                        include_summary: false,
                    },
                    Some(Arc::new(move |p| {
                        report(Progress {
                            phase: "export".into(),
                            done: p.completed as u64,
                            total: Some(p.total as u64),
                        })
                    })),
                )
                .await?;
            done(Exported { path })
        }
    }
}

fn item(engine: &Engine, id: &str) -> Outcome<ItemRow> {
    engine
        .item(id)?
        .ok_or_else(|| ControlError::not_found("item", id))
}

fn series(engine: &Engine, id: &str) -> Outcome<eddy_core::SeriesInfo> {
    engine
        .series_info(id)?
        .ok_or_else(|| ControlError::not_found("series", id))
}

fn chapters(engine: &Engine, series: &str) -> Outcome<Vec<ItemRow>> {
    Ok(engine.items(&ItemQuery {
        filter: ItemFilter::Series(series.to_string()),
        limit: ALL_CHAPTERS,
        ..Default::default()
    })?)
}

/// A tag's ID from its name (or from the ID itself).
fn tag(engine: &Engine, name: &str) -> Outcome<String> {
    let name = name.trim();
    engine
        .tags()?
        .into_iter()
        .find(|t| t.name == name || t.id == name)
        .map(|t| t.id)
        .ok_or_else(|| ControlError::new("not_found", format!("no tag named {name}")))
}

fn filter(engine: &Engine, scope: &Scope) -> Outcome<ItemFilter> {
    Ok(match scope {
        Scope::All => ItemFilter::All,
        Scope::Starred => ItemFilter::Starred,
        Scope::Saved => ItemFilter::Saved,
        Scope::Continue => ItemFilter::ContinueReading,
        Scope::Feed(id) => ItemFilter::Source(id.clone()),
        Scope::Folder(name) => ItemFilter::Category(name.clone()),
        Scope::Series(id) => ItemFilter::Series(id.clone()),
        Scope::Tag(name) => ItemFilter::Tag(tag(engine, name)?),
        Scope::Title(text) => ItemFilter::Title(text.clone()),
    })
}

fn feeds(engine: &Engine) -> Outcome<Vec<FeedDto>> {
    let stats = engine.source_overview()?;
    Ok(engine
        .sources()?
        .iter()
        .filter(|s| s.source.kind == SourceKind::Feed)
        .map(|s| FeedDto::new(s, stats.iter().find(|t| t.source_id == s.source.id)))
        .collect())
}

fn reading(engine: &Engine, id: &str) -> Outcome<ProgressDto> {
    let row = item(engine, id)?;
    let resume = engine.resume_point(id)?;
    Ok(ProgressDto::new(&row, resume.as_ref()))
}

/// The item's document, fetched when it has a page but no text yet (a
/// chapter of a web series that was never opened).
async fn document(engine: &Engine, row: &ItemRow) -> Outcome<Document> {
    let id = &row.item.id;
    if let Some(loaded) = engine.document(id).await? {
        return Ok(loaded.document);
    }
    if row.item.url.is_some()
        && let Ok(loaded) = engine.fetch_full_text(id).await
    {
        return Ok(loaded.document);
    }
    Err(ControlError::new(
        "no_content",
        format!("{id} has no content on this device"),
    ))
}

fn doc(
    row: &ItemRow,
    document: &Document,
    format: DocFormat,
    pages: Option<(u32, u32)>,
) -> Outcome<DocDto> {
    let count = document.pages.len() as u32;
    let range = match pages {
        Some((first, last)) => {
            let found = (first >= 1 && first <= last)
                .then(|| document.page_blocks(first as usize - 1..=last as usize - 1))
                .flatten();
            Some(found.ok_or_else(|| {
                ControlError::invalid(match count {
                    0 => "the item has no pages: it is not a PDF".to_string(),
                    n => format!("pages {first}-{last} are outside the PDF's {n} pages"),
                })
            })?)
        }
        None => None,
    };
    let blocks = match &range {
        Some(range) => &document.blocks[range.clone()],
        None => &document.blocks[..],
    };
    let mut content = None;
    let mut model = None;
    match format {
        DocFormat::Markdown => {
            content = Some(document.to_markdown(&MarkdownOptions {
                // The whole text carries its title; a run of pages does not.
                title: range.is_none().then_some(row.item.title.as_str()),
                blocks: range.clone(),
            }));
        }
        DocFormat::Text => {
            let text: Vec<String> = blocks
                .iter()
                .map(|b| b.plain_text())
                .filter(|t| !t.is_empty())
                .collect();
            content = Some(text.join("\n\n"));
        }
        DocFormat::Blocks => {
            let lines: Vec<String> = blocks
                .iter()
                .map(|b| {
                    let tag = match &b.kind {
                        BlockKind::Heading { level, .. } => format!("h{level}"),
                        BlockKind::Image { src, .. } => format!("image {src}"),
                        other => other.tag().to_string(),
                    };
                    format!("{}  [{tag}] {}", b.id, b.plain_text().replace('\n', " "))
                })
                .collect();
            content = Some(lines.join("\n"));
        }
        DocFormat::Summary => content = Some(summary(document)),
        DocFormat::Json => {
            model = Some(serde_json::to_value(Document {
                schema_version: document.schema_version,
                blocks: blocks.to_vec(),
                pages: document.pages.clone(),
            })?);
        }
    }
    Ok(DocDto {
        id: row.item.id.clone(),
        title: row.item.title.clone(),
        url: row.item.url.clone(),
        format: serde_json::to_value(format)?
            .as_str()
            .unwrap_or_default()
            .to_string(),
        pages: (count > 0).then_some(count),
        text_len: document.text_len() as u32,
        content,
        document: model,
    })
}

/// What a document is made of, for checking how a page or PDF came out.
fn summary(document: &Document) -> String {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for block in &document.blocks {
        *counts.entry(block.kind.tag()).or_insert(0) += 1;
    }
    let mut out = format!("blocks: {counts:?}, text_len: {}", document.text_len());
    for block in document
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
        .take(3)
    {
        out.push_str(&format!("\n  math block: {}", block.plain_text()));
    }
    let inline: Vec<&str> = document
        .blocks
        .iter()
        .flat_map(|b| b.kind.runs().unwrap_or(&[]))
        .filter(|r| r.math)
        .map(|r| r.text.as_str())
        .take(3)
        .collect();
    if !inline.is_empty() {
        out.push_str(&format!("\n  inline math: {inline:?}"));
    }
    out.push_str(&format!("\n  excerpt: {}", document.excerpt(160)));
    out
}

async fn status(engine: &Engine) -> Outcome<StatusDto> {
    // Asking the server first is what tells the engine whether it is
    // reachable.
    let asked = engine.server_status().await;
    let status = engine.status()?;
    let server = (status.mode == Mode::Server).then(|| {
        let (info, error) = match asked {
            Ok(info) => (info, None),
            Err(e) => (None, Some(e.to_string())),
        };
        ServerDto {
            online: status.server_online,
            url: info.as_ref().map(|s| s.url.clone()),
            version: info.as_ref().map(|s| s.version.clone()),
            cursor: info.as_ref().map(|s| s.cursor),
            error,
            pending_uploads: status.pending_uploads,
            rejected_uploads: status.rejected_uploads,
            offline_refresh: status.offline_refresh,
        }
    });
    Ok(StatusDto {
        mode: match status.mode {
            Mode::Unconfigured => "unconfigured",
            Mode::Standalone => "standalone",
            Mode::Server => "server",
        }
        .to_string(),
        account: status.account.map(|a| AccountDto {
            url: a.base_url,
            username: a.username,
        }),
        last_refresh: status.last_refresh,
        last_error: status.last_error,
        server,
        core: CORE_VERSION.to_string(),
        sync_protocol: SYNC_PROTOCOL_VERSION,
    })
}

async fn sync(engine: &Engine, history: bool, progress: ProgressFn) -> Outcome<SyncDto> {
    // The engine has one progress listener and the app owns it, so how far
    // a long sync is comes from looking.
    let refresh = engine.refresh();
    tokio::pin!(refresh);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let summary = loop {
        tokio::select! {
            result = &mut refresh => break result?,
            _ = tick.tick() => {
                if let Ok(Some(sync)) = engine.status().map(|s| s.sync) {
                    progress(Progress {
                        phase: match sync.phase {
                            SyncPhase::Uploading => "uploading",
                            SyncPhase::Downloading => "downloading",
                            SyncPhase::FetchingContent => "fetching",
                        }
                        .to_string(),
                        done: sync.done,
                        total: sync.total,
                    });
                }
            }
        }
    };
    let mut older = 0;
    if history {
        let report = progress.clone();
        older = engine
            .backfill_history(Some(Arc::new(move |added| {
                report(Progress {
                    phase: "history".into(),
                    done: u64::from(added),
                    total: None,
                });
            })))
            .await?;
    }
    Ok(SyncDto {
        new_items: summary.new_items,
        state_changes: summary.state_changes,
        offline: summary.offline,
        history: older,
    })
}
