use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use eddy_model::{
    Account, ContentOrigin, FullTextMode, Item, ItemKind, Source, SourceKind, fields, ids, now_ms,
};
use eddy_source_greader::{GReaderClient, READ, READING_LIST, STARRED, UpstreamItem, history_path};

use crate::engine::{Inner, RefreshSummary};
use crate::{LogLevel, Result};

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

    /// Downloads a content blob that is not available locally.
    async fn fetch_blob(&self, _key: &str) -> Result<Option<(Vec<u8>, String)>> {
        Ok(None)
    }
}

/// Standalone mode: a FreshRSS (Google Reader API) account accessed directly.
pub(crate) struct DirectUpstream {
    pub account: Account,
    pub client: GReaderClient,
}

const ID_CAP: usize = 50_000;
const MISSING_FETCH_CAP: usize = 1_000;
/// Older reading-list pages (100 items each) backfilled per sync.
const HISTORY_PAGES_PER_SYNC: usize = 10;

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
            let Some(value) = op
                .value
                .as_bool()
                .filter(|_| op.field == fields::READ || op.field == fields::STARRED)
            else {
                unrelated.push(stored.seq);
                continue;
            };
            let entry = latest
                .entry((op.entity_id.clone(), op.field.clone()))
                .or_insert((value, Vec::new()));
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
        let previous: HashMap<String, Source> = inner
            .store
            .sources()?
            .into_iter()
            .filter(|s| s.account_id.as_deref() == Some(account_id))
            .map(|s| (s.id.clone(), s))
            .collect();
        let mut source_ids = Vec::new();
        let mut feed_to_source = HashMap::new();
        for sub in &subscriptions {
            let id = ids::feed_source_id(account_id, &sub.id);
            let source = Source {
                id: id.clone(),
                account_id: Some(account_id.to_string()),
                upstream_id: Some(sub.id.clone()),
                kind: SourceKind::Feed,
                title: sub.title.clone(),
                category: sub.category.clone(),
                site_url: sub.site_url.clone(),
                full_text: FullTextMode::Auto,
                truncated: false,
            };
            inner.store.upsert_source(&source)?;
            let changed = previous.get(&id).is_none_or(|p| {
                p.title != source.title
                    || p.category != source.category
                    || p.site_url != source.site_url
            });
            if changed {
                inner.publish(
                    eddy_model::Entity::Source,
                    &id,
                    fields::CREATE,
                    serde_json::to_value(&source)?,
                )?;
            }
            feed_to_source.insert(sub.id.clone(), id.clone());
            source_ids.push(id);
        }
        for gone in previous.keys().filter(|id| !source_ids.contains(id)) {
            inner.publish(
                eddy_model::Entity::Source,
                gone,
                fields::DELETED,
                true.into(),
            )?;
        }
        inner.store.remove_sources_not_in(account_id, &source_ids)?;

        let unread: HashSet<String> = self
            .client
            .item_ids(READING_LIST, Some(READ), ID_CAP)
            .await?
            .into_iter()
            .collect();
        let starred: HashSet<String> = self
            .client
            .item_ids(STARRED, None, ID_CAP)
            .await?
            .into_iter()
            .collect();
        let known: HashMap<String, String> = inner
            .store
            .upstream_items(account_id)?
            .into_iter()
            .map(|(id, up, _, _)| (up, id))
            .collect();

        let recent = self
            .client
            .stream_page(&history_path("all", false, None)?, None)
            .await?;
        let mut incoming: Vec<UpstreamItem> = recent
            .items
            .into_iter()
            .filter(|i| !known.contains_key(&i.id))
            .collect();
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

        let mut seen: HashSet<String> = known.into_keys().collect();
        let mut new_item_ids = Vec::new();
        for up in incoming {
            seen.insert(up.id.clone());
            let source_id = feed_to_source.get(&up.feed_id).cloned();
            if let Some(id) = self.ingest(inner, &up, source_id)? {
                new_item_ids.push(id);
            }
        }
        self.backfill_history(inner, &feed_to_source, &mut seen, recent.continuation)
            .await?;

        let mut state_changes = 0;
        for (id, up, _, _) in inner.store.upstream_items(account_id)? {
            for (field, value) in [
                (fields::READ, !unread.contains(&up)),
                (fields::STARRED, starred.contains(&up)),
            ] {
                let changed = if inner.options.server_role {
                    // On a server, upstream changes become ops so they reach clients.
                    let differs = inner.store.upstream_state_differs(&id, field, value)?;
                    if differs {
                        inner.record(eddy_model::Entity::Item, &id, field, value.into(), false)?;
                    }
                    differs
                } else {
                    inner.store.set_state_from_upstream(&id, field, value)?
                };
                if changed {
                    state_changes += 1;
                }
            }
        }

        if let Some(e) = push_error {
            return Err(e);
        }
        Ok(RefreshSummary {
            new_items: new_item_ids.len() as u32,
            state_changes,
            sources: subscriptions.len() as u32,
            new_item_ids,
        })
    }
}

impl DirectUpstream {
    /// Walks the reading list past its first page, a few pages per sync, so
    /// articles already read upstream become local too. They are history, not
    /// new arrivals, so automatic processing skips them. A failed page is
    /// retried on the next sync from the saved cursor.
    async fn backfill_history(
        &self,
        inner: &Inner,
        feed_to_source: &HashMap<String, String>,
        seen: &mut HashSet<String>,
        after_first_page: Option<String>,
    ) -> Result<()> {
        // An empty value marks the end: it is never a valid continuation.
        let key = format!("history_cursor:{}", self.account.id);
        let mut cursor = match inner.store.meta_get(&key)? {
            Some(saved) if saved.is_empty() => return Ok(()),
            Some(saved) => saved,
            None => match after_first_page {
                Some(next) => next,
                None => {
                    inner.store.meta_set(&key, "")?;
                    return Ok(());
                }
            },
        };
        for _ in 0..HISTORY_PAGES_PER_SYNC {
            let page: eddy_source_greader::Result<_> = async {
                let path = history_path("all", false, Some(&cursor))?;
                self.client.stream_page(&path, Some(&cursor)).await
            }
            .await;
            let page = match page {
                Ok(page) => page,
                Err(e) => {
                    inner
                        .platform
                        .log(LogLevel::Warn, &format!("history backfill paused: {e}"));
                    return Ok(());
                }
            };
            for up in page.items {
                if seen.insert(up.id.clone()) {
                    let source_id = feed_to_source.get(&up.feed_id).cloned();
                    self.ingest(inner, &up, source_id)?;
                }
            }
            match page.continuation {
                Some(next) => {
                    inner.store.meta_set(&key, &next)?;
                    cursor = next;
                }
                None => {
                    inner.store.meta_set(&key, "")?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// Stores a new upstream item. Returns its ID when it was new.
    fn ingest(
        &self,
        inner: &Inner,
        up: &UpstreamItem,
        source_id: Option<String>,
    ) -> Result<Option<String>> {
        let account_id = self.account.id.as_str();
        let item_id = ids::feed_item_id(account_id, &up.id);
        let published = if up.published_ms > 0 {
            up.published_ms
        } else {
            now_ms()
        };
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
            inner.store.update_item_meta(
                &item_id,
                &up.title,
                up.url.as_deref(),
                up.author.as_deref(),
            )?;
            return Ok(None);
        }
        let ingested = inner.ingest_html(
            &item_id,
            &up.html,
            ContentOrigin::Feed,
            up.url.as_deref(),
            Some(&up.title),
            false,
        )?;
        inner.store.set_current_version(
            &item_id,
            &ingested.version.hash,
            &ingested.excerpt,
            ingested.version.text_len,
            ingested.lang.as_deref(),
        )?;
        if inner.options.server_role {
            inner.publish(
                eddy_model::Entity::Item,
                &item_id,
                fields::CREATE,
                serde_json::to_value(&item)?,
            )?;
            inner.publish(
                eddy_model::Entity::ContentVersion,
                &ingested.version.hash,
                fields::CREATE,
                serde_json::to_value(&ingested.version)?,
            )?;
            inner.publish(
                eddy_model::Entity::Item,
                &item_id,
                fields::VERSION,
                ingested.version.hash.clone().into(),
            )?;
            inner.publish(
                eddy_model::Entity::Item,
                &item_id,
                "meta",
                ingested.meta_value(),
            )?;
        }
        inner.after_ingest(&item_id, &ingested);
        Ok(Some(item_id))
    }

    /// Re-downloads feed content whose raw source was evicted from the cache.
    pub(crate) async fn refetch(
        &self,
        inner: &Inner,
        item_id: &str,
        upstream_id: &str,
    ) -> Result<bool> {
        let items = self.client.contents(&[upstream_id.to_string()]).await?;
        let Some(up) = items.into_iter().next() else {
            return Ok(false);
        };
        let ingested = inner.ingest_html(
            item_id,
            &up.html,
            ContentOrigin::Feed,
            up.url.as_deref(),
            Some(&up.title),
            false,
        )?;
        if inner
            .store
            .item(item_id)?
            .and_then(|r| r.item.current_version)
            .is_none_or(|v| v == ingested.version.hash)
        {
            inner.store.set_current_version(
                item_id,
                &ingested.version.hash,
                &ingested.excerpt,
                ingested.version.text_len,
                ingested.lang.as_deref(),
            )?;
        }
        Ok(true)
    }
}
