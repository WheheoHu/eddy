//! Headless developer tool around the engine. Uses its own data directory
//! (default `./build/cli-data`) with a plain-file keychain inside it, so it
//! never touches the app's data or the system keychain.
//!
//! eddy-cli [--data DIR] save URL
//! eddy-cli [--data DIR] items
//! eddy-cli [--data DIR] doc ITEM_ID
//! eddy-cli [--data DIR] text ITEM_ID
//! eddy-cli [--data DIR] fulltext ITEM_ID
//! eddy-cli [--data DIR] import FILE.epub|FILE.pdf
//! eddy-cli [--data DIR] connect URL CODE | disconnect | sync | status
//! eddy-cli [--data DIR] freshrss URL USER PASSWORD
//! eddy-cli [--data DIR] server-account URL USER PASSWORD
//! eddy-cli [--data DIR] tag NAME | tags | star ITEM_ID
//! eddy-cli --version

use std::path::PathBuf;
use std::sync::Arc;

use eddy_core::{Engine, EngineOptions, GalleryScope, ItemFilter, ItemQuery, Platform};
use eddy_model::{BlockKind, Document};

/// Development-only keychain: secrets in `cli-secrets.json` (mode 0600)
/// inside the CLI's own data directory.
struct FileSecrets(PathBuf);

impl FileSecrets {
    fn load(&self) -> std::collections::HashMap<String, String> {
        std::fs::read(&self.0)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, map: &std::collections::HashMap<String, String>) -> bool {
        use std::os::unix::fs::OpenOptionsExt;
        let Ok(bytes) = serde_json::to_vec(map) else {
            return false;
        };
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.0)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &bytes))
            .is_ok()
    }
}

impl Platform for FileSecrets {
    fn secret_get(&self, key: &str) -> Option<String> {
        self.load().get(key).cloned()
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        let mut map = self.load();
        map.insert(key.into(), value.into());
        self.save(&map)
    }

    fn secret_delete(&self, key: &str) {
        let mut map = self.load();
        map.remove(key);
        self.save(&map);
    }
}

fn summarize(doc: &Document) {
    let mut counts = std::collections::BTreeMap::new();
    for b in &doc.blocks {
        *counts.entry(b.kind.tag()).or_insert(0) += 1;
    }
    println!("blocks: {counts:?}, text_len: {}", doc.text_len());
    for b in doc
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Math { .. }))
        .take(3)
    {
        println!("  math block: {}", b.plain_text());
    }
    let inline: Vec<_> = doc
        .blocks
        .iter()
        .flat_map(|b| {
            b.kind
                .runs()
                .unwrap_or(&[])
                .iter()
                .filter(|r| r.math)
                .map(|r| r.text.clone())
        })
        .take(3)
        .collect();
    if !inline.is_empty() {
        println!("  inline math: {inline:?}");
    }
    println!("  excerpt: {}", doc.excerpt(160));
}

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!(
            "eddy-cli {} (sync protocol {})",
            eddy_core::core_version(),
            eddy_core::SYNC_PROTOCOL_VERSION
        );
        return;
    }
    let mut data = PathBuf::from("build/cli-data");
    if args.first().map(String::as_str) == Some("--data") && args.len() > 1 {
        data = PathBuf::from(args.remove(1));
        args.remove(0);
    }
    std::fs::create_dir_all(&data).expect("data dir");
    let secrets = Arc::new(FileSecrets(data.join("cli-secrets.json")));
    let engine = Engine::open(&data, secrets, EngineOptions::default()).expect("open engine");
    engine.set_pdf_reader(Some(Arc::new(eddy_pdf::PdfiumReader::new())));
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["save", url] => engine.save_url(url).await.map(|row| {
            println!("saved {} — {}", row.item.id, row.item.title);
        }),
        ["items"] => engine.items(&ItemQuery::default()).map(|rows| {
            for r in rows {
                println!("{}  {}  {}", r.item.id, r.item.kind.as_str(), r.item.title);
            }
        }),
        ["doc", id] => engine.document(id).await.map(|d| match d {
            Some(d) => summarize(&d.document),
            None => println!("no content"),
        }),
        // Every block as text, one per line prefixed with its kind; for
        // checking how a PDF or page reflowed.
        ["text", id] => engine.document(id).await.map(|d| match d {
            Some(d) => {
                for b in &d.document.blocks {
                    let tag = match &b.kind {
                        BlockKind::Heading { level, .. } => format!("h{level}"),
                        BlockKind::Image { src, .. } => format!("image {src}"),
                        other => other.tag().to_string(),
                    };
                    println!("[{tag}] {}", b.plain_text());
                }
            }
            None => println!("no content"),
        }),
        ["fulltext", id] => engine
            .fetch_full_text(id)
            .await
            .map(|d| summarize(&d.document)),
        ["connect", url, code] => engine.connect_server(url, code, "eddy-cli").await.map(|s| {
            println!(
                "connected to {} (server {}), cursor {}",
                s.url, s.server_id, s.cursor
            );
        }),
        ["disconnect"] => engine.disconnect_server(),
        // Standalone: this device's FreshRSS account. Server mode: the same
        // account, kept here for refreshing while the server is offline.
        ["freshrss", url, user, password] => engine
            .configure_freshrss(url, user, password)
            .await
            .map(|a| println!("FreshRSS {} @ {}", a.username, a.base_url)),
        // The FreshRSS account the server manages.
        ["server-account", url, user, password] => engine
            .configure_server_freshrss(url, user, password)
            .await
            .map(|_| println!("server account set")),
        ["status"] => {
            // Asking the server first is what tells the engine whether it
            // is reachable.
            let server = engine.server_status().await;
            match engine.status() {
                Ok(s) => {
                    println!("mode {:?}", s.mode);
                    if s.mode == eddy_core::Mode::Server {
                        println!(
                            "server {}",
                            if s.server_online { "online" } else { "offline" }
                        );
                        match server {
                            Ok(Some(server)) => println!(
                                "server version {}, cursor {}",
                                server.version, server.cursor
                            ),
                            Ok(None) => {}
                            Err(e) => println!("server error: {e}"),
                        }
                        println!("pending uploads {}", s.pending_uploads);
                        println!("refused uploads {}", s.rejected_uploads);
                        println!("offline refresh {}", s.offline_refresh);
                    }
                    if let Some(e) = s.last_error {
                        println!("last error {e}");
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        ["tag", name] => engine.create_tag(name).map(|id| println!("tag {id}")),
        ["tags"] => engine.tags().map(|tags| {
            for t in tags {
                println!("{}", t.name);
            }
        }),
        ["star", id] => engine.set_starred(id, true).await,
        ["series", url] => engine.detect_series_at(url).await.map(|p| match p {
            Some(p) => {
                println!("{} — {} chapters", p.title, p.chapters.len());
                for c in &p.chapters {
                    let indent = "  ".repeat(c.level as usize + 1);
                    println!("{indent}{}  {}", c.title, c.url);
                }
            }
            None => println!("no table of contents found"),
        }),
        ["sync"] => match engine.refresh().await {
            Ok(s) => {
                println!(
                    "new items {}, state changes {}{}",
                    s.new_items,
                    s.state_changes,
                    if s.offline {
                        " (server offline: fetched from FreshRSS)"
                    } else {
                        ""
                    }
                );
                let progress: eddy_core::HistoryCallback =
                    Arc::new(|added| println!("history: {added} older articles"));
                engine.backfill_history(Some(progress)).await.map(|_| ())
            }
            Err(e) => Err(e),
        },
        ["search", query] => engine.search_all(query, 10).await.map(|hits| {
            for h in hits {
                println!("{}  {}  …{}…", h.item.item.id, h.item.item.title, h.snippet);
            }
        }),
        ["import", path] => match engine.import_file(std::path::Path::new(path)).await {
            Ok(imported) => {
                let verb = if imported.existing {
                    "already imported"
                } else {
                    "imported"
                };
                println!("{verb} {} — {}", imported.item_id, imported.title);
                let chapters = match &imported.series_id {
                    Some(series) => engine.items(&ItemQuery {
                        filter: ItemFilter::Series(series.clone()),
                        limit: 10_000,
                        ..Default::default()
                    }),
                    None => Ok(Vec::new()),
                };
                chapters.map(|rows| {
                    for r in rows {
                        println!("  {:>6} chars  {}", r.item.text_len, r.item.title);
                    }
                })
            }
            Err(e) => Err(e),
        },
        ["media", id] => engine.resolve_media(id, true).await.map(|set| {
            println!("{:?}  thumbnail {:?}", set.status, set.thumbnail);
            for m in set.items {
                println!(
                    "  {:?} {}  poster {:?}  {:?}x{:?}  {:?}s",
                    m.kind, m.src, m.poster, m.width, m.height, m.duration
                );
            }
        }),
        ["gallery", folder, count] => {
            let count: u32 = count.parse().unwrap_or(20);
            match engine.set_folder_gallery(folder, true).and_then(|_| {
                engine.gallery_items(&GalleryScope::Folder(folder.to_string()), false, count, 0)
            }) {
                Ok(entries) => {
                    let mut tally = std::collections::BTreeMap::new();
                    for e in entries {
                        let host = e
                            .row
                            .item
                            .url
                            .as_deref()
                            .and_then(|u| url::Url::parse(u).ok())
                            .and_then(|u| u.host_str().map(String::from))
                            .unwrap_or_default();
                        let set = engine.resolve_media(&e.row.item.id, true).await;
                        let key = match &set {
                            Ok(s) => format!(
                                "{host:<22} {:?} {:?}",
                                s.status,
                                s.items.first().map(|m| m.kind)
                            ),
                            Err(err) => format!("{host:<22} error {err}"),
                        };
                        *tally.entry(key).or_insert(0) += 1;
                    }
                    for (k, n) in tally {
                        println!("{n:>4}  {k}");
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ => {
            eprintln!(
                "usage: eddy-cli [--data DIR] save URL | items | doc ID | text ID | fulltext ID | connect URL CODE | disconnect | sync | status | freshrss URL USER PASSWORD | server-account URL USER PASSWORD | tag NAME | tags | star ID | search QUERY | import FILE | media ID | gallery FOLDER COUNT | --version"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
