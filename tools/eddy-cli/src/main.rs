//! Headless developer tool around the engine. Uses its own data directory
//! (default `./build/cli-data`) with a plain-file keychain inside it, so it
//! never touches the app's data or the system keychain.
//!
//! eddy-cli [--data DIR] save URL
//! eddy-cli [--data DIR] items
//! eddy-cli [--data DIR] doc ITEM_ID
//! eddy-cli [--data DIR] fulltext ITEM_ID

use std::path::PathBuf;
use std::sync::Arc;

use eddy_core::{Engine, EngineOptions, ItemQuery, Platform};
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
    let mut data = PathBuf::from("build/cli-data");
    if args.first().map(String::as_str) == Some("--data") && args.len() > 1 {
        data = PathBuf::from(args.remove(1));
        args.remove(0);
    }
    std::fs::create_dir_all(&data).expect("data dir");
    let secrets = Arc::new(FileSecrets(data.join("cli-secrets.json")));
    let engine = Engine::open(&data, secrets, EngineOptions::default()).expect("open engine");
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
        ["series", url] => engine.detect_series_at(url).await.map(|p| match p {
            Some(p) => {
                println!("{} — {} chapters", p.title, p.chapters.len());
                for c in p.chapters.iter().take(5) {
                    println!("  {}  {}", c.title, c.url);
                }
            }
            None => println!("no table of contents found"),
        }),
        ["sync"] => engine.refresh().await.map(|s| {
            println!(
                "new items {}, state changes {}",
                s.new_items, s.state_changes
            )
        }),
        ["search", query] => engine.search_all(query, 10).await.map(|hits| {
            for h in hits {
                println!("{}  {}  …{}…", h.item.item.id, h.item.item.title, h.snippet);
            }
        }),
        _ => {
            eprintln!(
                "usage: eddy-cli [--data DIR] save URL | items | doc ID | fulltext ID | connect URL CODE | sync | search QUERY"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
