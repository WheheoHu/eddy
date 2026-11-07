use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use eddy_model::{
    Account, ContentOrigin, FullTextMode, Item, ItemKind, Source, SourceKind, fields, ids, now_ms,
};
use eddy_source_greader::{GReaderClient, READ, READING_LIST, STARRED, UpstreamItem, history_path};

use crate::engine::{Inner, RefreshSummary};
use crate::Result;

/// Where the engine gets content and sends local changes. Standalone mode
/// talks to FreshRSS directly; server mode syncs with `eddy-server`. Every
/// other part of the engine is unaware of which one is active.
#[async_trait]
pub(crate) trait Upstream: Send + Sync {
    fn kind(&self) -> &'static str;

    /// Pushes pending local changes, then pulls new content and state.
    async fn sync(&self, inner: &Inner) -> Result<RefreshSummary>;

    /// Pushes pending local changes only (after a read/star toggle).
    async fn push(&self, inner: &Inner) -> Result<()>;
}

/// Standalone mode: a FreshRSS (Google Reader API) account accessed directly.
pub(crate) struct DirectUpstream {
    pub account: Account,
    pub client: GReaderClient,
}

const ID_CAP: usize = 50_000;
const MISSING_FETCH_CAP: usize = 1_000;

#[async_trait]
impl Upstream for DirectUpstream {
    fn kind(&self) -> &'static str {
        "direct"
    }

    async fn push(&self, inner: &Inner) -> Result<()> {
        let pending = inner.store.upstream_pending()?;
        if pending.is_empty() {
            return Ok(());
        }
        // Collapse to the latest value per (item, field), then batch by value.
        let mut latest: HashMap<(String, String), (bool, Vec<i64>)> = HashMap::new();
        let mut unrelated = Vec::new();
        for stored in pending {
            let op = &stored.op;
            let Some(value) = op.value.as_bool().filter(|_| op.field == fields::READ || op.field == fields::STARRED) else {
                unrelated.push(stored.seq);
                continue;
            };
            let entry = latest.entry((op.entity_id.clone(), op.field.clone())).or_insert((value, Vec::new()));
            entry.0 = value;
            entry.1.push(stored.seq);
        }
        inner.store.clear_upstream_pending(&unrelated)?;
        let mut batches: HashMap<(String, bool), (Vec<String>, Vec<i64>)> = HashMap::new();
        for ((item_id, field), (value, seqs)) in latest {
            let upstream_id = inner
                .store
                .item(&item_id)?
                .filter(|r| r.item.account_id.as_deref() == Some(self.account.id.as_str()))
                .and_then(|r| r.item.upstream_id);
            match upstream_id {
                Some(up) => {
                    let batch = batches.entry((field, value)).or_default();
                    batch.0.push(up);
                    batch.1.extend(seqs);
                }
                None => inner.store.clear_upstream_pending(&seqs)?,
            }
        }
        for ((field, value), (ids, seqs)) in batches {
            let tag = if field == fields::READ { READ } else { STARRED };
            self.client.edit_tag(&ids, tag, value).await?;
            inner.store.clear_upstream_pending(&seqs)?;
        }
        Ok(())
    }

    async fn sync(&self, inner: &Inner) -> Result<RefreshSummary> {
        let account_id = self.account.id.as_str();
        // Local changes first, so upstream state already includes them.
        let push_error = self.push(inner).await.err();

        let subscriptions = self.client.subscriptions().await?;
        let mut source_ids = Vec::new();
        let mut feed_to_source = HashMap::new();
        for sub in &subscriptions {
            let id = ids::feed_source_id(account_id, &sub.id);
            inner.store.upsert_source(&Source {
                id: id.clone(),
                account_id: Some(account_id.to_string()),
                upstream_id: Some(sub.id.clone()),
                kind: SourceKind::Feed,
                title: sub.title.clone(),
                category: sub.category.clone(),
                site_url: sub.site_url.clone(),
                full_text: FullTextMode::Auto,
                truncated: false,
            })?;
            feed_to_source.insert(sub.id.clone(), id.clone());
            source_ids.push(id);
        }
        inner.store.remove_sources_not_in(account_id, &source_ids)?;

        let unread: HashSet<String> = self.client.item_ids(READING_LIST, Some(READ), ID_CAP).await?.into_iter().collect();
        let starred: HashSet<String> = self.client.item_ids(STARRED, None, ID_CAP).await?.into_iter().collect();
        let known: HashMap<String, String> =
            inner.store.upstream_items(account_id)?.into_iter().map(|(id, up, _, _)| (up, id)).collect();

        let recent = self.client.stream_page(&history_path("all", false, None)?, None).await?;
        let mut incoming: Vec<UpstreamItem> = recent.items.into_iter().filter(|i| !known.contains_key(&i.id)).collect();
        let fetched: HashSet<String> = incoming.iter().map(|i| i.id.clone()).collect();
        let mut missing: Vec<String> = unread
            .iter()
            .chain(starred.iter())
            .filter(|id| !known.contains_key(*id) && !fetched.contains(*id))
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        // Hex IDs from FreshRSS grow over time; fetch the newest first.
        missing.sort_unstable_by(|a, b| b.cmp(a));
        missing.truncate(MISSING_FETCH_CAP);
        incoming.extend(self.client.contents(&missing).await?);

        let mut new_items = 0;
        for up in incoming {
            let source_id = feed_to_source.get(&up.feed_id).cloned();
            if self.ingest(inner, &up, source_id)? {
                new_items += 1;
            }
        }

        let mut state_changes = 0;
        for (id, up, _, _) in inner.store.upstream_items(account_id)? {
            if inner.store.set_state_from_upstream(&id, fields::READ, !unread.contains(&up))? {
                state_changes += 1;
            }
            if inner.store.set_state_from_upstream(&id, fields::STARRED, starred.contains(&up))? {
                state_changes += 1;
            }
        }

        if let Some(e) = push_error {
            return Err(e);
        }
        Ok(RefreshSummary { new_items, state_changes, sources: subscriptions.len() as u32 })
    }
}

impl DirectUpstream {
    fn ingest(&self, inner: &Inner, up: &UpstreamItem, source_id: Option<String>) -> Result<bool> {
        let account_id = self.account.id.as_str();
        let item_id = ids::feed_item_id(account_id, &up.id);
        let published = if up.published_ms > 0 { up.published_ms } else { now_ms() };
        let item = Item {
            id: item_id.clone(),
            source_id,
            account_id: Some(account_id.to_string()),
            upstream_id: Some(up.id.clone()),
            kind: ItemKind::Feed,
            url: up.url.clone(),
            title: up.title.clone(),
            author: up.author.clone(),
            published_at: published,
            added_at: now_ms(),
            read: up.read,
            starred: up.starred,
            series_id: None,
            chapter_index: None,
            current_version: None,
            excerpt: None,
            lang: None,
            text_len: 0,
            progress: 0.0,
            finished: false,
            last_read_at: None,
        };
        if !inner.store.insert_item(&item)? {
            inner.store.update_item_meta(&item_id, &up.title, up.url.as_deref(), up.author.as_deref())?;
            return Ok(false);
        }
        let ingested = inner.ingest_html(&item_id, &up.html, ContentOrigin::Feed, up.url.as_deref(), Some(&up.title), false)?;
        inner.store.set_current_version(&item_id, &ingested.version.hash, &ingested.excerpt, ingested.version.text_len, ingested.lang.as_deref())?;
        inner.after_ingest(&item_id, &ingested);
        Ok(true)
    }

    /// Re-downloads feed content whose raw source was evicted from the cache.
    pub(crate) async fn refetch(&self, inner: &Inner, item_id: &str, upstream_id: &str) -> Result<bool> {
        let items = self.client.contents(&[upstream_id.to_string()]).await?;
        let Some(up) = items.into_iter().next() else { return Ok(false) };
        let ingested = inner.ingest_html(item_id, &up.html, ContentOrigin::Feed, up.url.as_deref(), Some(&up.title), false)?;
        if inner.store.item(item_id)?.and_then(|r| r.item.current_version).is_none_or(|v| v == ingested.version.hash) {
            inner.store.set_current_version(item_id, &ingested.version.hash, &ingested.excerpt, ingested.version.text_len, ingested.lang.as_deref())?;
        }
        Ok(true)
    }
}
