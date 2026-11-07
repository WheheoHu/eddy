//! Headless developer tool around the engine. Uses its own data directory
//! (default `./build/cli-data`) and an in-memory keychain, so it never touches
//! the app's data or secrets.
//!
//! eddy-cli [--data DIR] save URL
//! eddy-cli [--data DIR] items
//! eddy-cli [--data DIR] doc ITEM_ID
//! eddy-cli [--data DIR] fulltext ITEM_ID

use std::path::PathBuf;
use std::sync::Arc;

use eddy_core::{Engine, EngineOptions, ItemQuery, MemoryPlatform};
use eddy_model::{BlockKind, Document};

fn summarize(doc: &Document) {
    let mut counts = std::collections::BTreeMap::new();
    for b in &doc.blocks {
        *counts.entry(b.kind.tag()).or_insert(0) += 1;
    }
    println!("blocks: {counts:?}, text_len: {}", doc.text_len());
    for b in doc.blocks.iter().filter(|b| matches!(b.kind, BlockKind::Math { .. })).take(3) {
        println!("  math block: {}", b.plain_text());
    }
    let inline: Vec<_> = doc
        .blocks
        .iter()
        .flat_map(|b| b.kind.runs().unwrap_or(&[]).iter().filter(|r| r.math).map(|r| r.text.clone()))
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
    let engine = Engine::open(&data, Arc::new(MemoryPlatform::default()), EngineOptions::default()).expect("open engine");
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
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
        ["fulltext", id] => engine.fetch_full_text(id).await.map(|d| summarize(&d.document)),
        _ => {
            eprintln!("usage: eddy-cli [--data DIR] save URL | items | doc ID | fulltext ID");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
