use std::time::{Duration, Instant};

use eddy_doc::{locator_at, remap_locator};
use eddy_extract::{ChapterLink, SeriesProposal, detect_series};
use eddy_model::{
    ContentOrigin, Entity, Item, ItemKind, Locator, PDF_MIME, Position, TextQuote, fields, ids,
    now_ms,
};
use eddy_store::{ItemFilter, ItemQuery, ItemRow};

use crate::engine::{Engine, Inner};
use crate::fulltext::PAGE_MIME;
use crate::{EngineError, Result};

/// Progression at which an item counts as finished.
const FINISHED_AT: f64 = 0.98;
/// Minimum interval between progress ops for one item while scrolling.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub struct ResumePoint {
    /// Locator re-anchored to the item's current content.
    pub locator: Locator,
    pub block_id: Option<String>,
    pub offset: u32,
    /// Page of a PDF (0-based).
    pub page: Option<u32>,
    pub updated_at: i64,
    /// Device that recorded it.
    pub device: Option<String>,
    pub from_other_device: bool,
    /// This device's own last position, offered as "back to where I was"
    /// when the resume point came from another device.
    pub local_block_id: Option<String>,
    pub local_offset: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesInfo {
    pub id: String,
    pub title: String,
    pub toc_url: Option<String>,
    pub author: Option<String>,
    /// Cover image URL, for `Engine::image`.
    pub cover: Option<String>,
    pub chapters: u32,
    pub finished: u32,
    /// Overall progress through the series, 0..=1.
    pub progress: f64,
    /// The chapter to continue with.
    pub next_item_id: Option<String>,
    /// The chapter to open: the one reading stopped in, or the next
    /// unfinished one after it; the first one when none was read.
    pub resume_item_id: Option<String>,
    pub created_at: i64,
    /// Latest publication date among the chapters.
    pub updated_at: i64,
    pub last_read_at: Option<i64>,
    /// Characters across all chapters.
    pub text_len: u32,
    pub lang: Option<String>,
}

impl Inner {
    pub(crate) fn chapters(&self, series_id: &str) -> Result<Vec<ItemRow>> {
        Ok(self.store.items(&ItemQuery {
            filter: ItemFilter::Series(series_id.into()),
            unread_only: false,
            keep: Vec::new(),
            limit: 10_000,
            offset: 0,
        })?)
    }

    pub(crate) fn series_info(&self, series: eddy_model::Series) -> Result<SeriesInfo> {
        let chapters = self.chapters(&series.id)?;
        let total = chapters.len() as u32;
        let finished = chapters.iter().filter(|r| r.item.finished).count() as u32;
        let partial: f64 = chapters
            .iter()
            .filter(|r| !r.item.finished)
            .map(|r| r.item.progress)
            .sum();
        let progress = if total == 0 {
            0.0
        } else {
            ((finished as f64 + partial) / total as f64).min(1.0)
        };
        // Continue where reading stopped: the first unfinished chapter after
        // the last one touched, else the first unfinished one.
        let last_touched = chapters
            .iter()
            .enumerate()
            .filter(|(_, r)| r.item.last_read_at.is_some())
            .max_by_key(|(_, r)| r.item.last_read_at)
            .map(|(i, _)| i);
        let next = last_touched
            .and_then(|i| chapters[i..].iter().find(|r| !r.item.finished))
            .or_else(|| chapters.iter().find(|r| !r.item.finished))
            .map(|r| r.item.id.clone());
        let resume = next
            .clone()
            .or_else(|| last_touched.map(|i| chapters[i].item.id.clone()))
            .or_else(|| chapters.first().map(|r| r.item.id.clone()));
        Ok(SeriesInfo {
            id: series.id,
            title: series.title,
            toc_url: series.toc_url,
            author: series.author,
            cover: series.cover,
            chapters: total,
            finished,
            progress,
            next_item_id: next,
            resume_item_id: resume,
            created_at: series.created_at,
            updated_at: chapters
                .iter()
                .map(|r| r.item.published_at)
                .max()
                .unwrap_or(series.created_at),
            last_read_at: chapters.iter().filter_map(|r| r.item.last_read_at).max(),
            text_len: chapters.iter().map(|r| r.item.text_len).sum(),
            lang: chapters.iter().find_map(|r| r.item.lang.clone()),
        })
    }

    /// Fetches content for items that have only a URL (series chapters).
    pub(crate) async fn ensure_page_content(&self, item_id: &str) -> Result<bool> {
        let row = self.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        if row.item.current_version.is_some() || row.item.kind == ItemKind::Feed {
            return Ok(false);
        }
        let Some(url) = row.item.url.clone() else {
            return Ok(false);
        };
        let fetcher = self.fetcher.read().clone();
        let page = fetcher
            .fetch(&url, |_| true)
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        let (ingested, _) = self.ingest_page(
            item_id,
            &page.html,
            &page.final_url,
            ContentOrigin::Extracted,
            true,
        )?;
        self.switch_version(item_id, &ingested)?;
        self.after_ingest(item_id, &ingested);
        Ok(true)
    }
}

impl Engine {
    /// Records the reading position reported by a renderer (top visible
    /// block and UTF-16 offset in it). `at_end` says the end of the text is
    /// on screen, which the top block alone can't show: a short article never
    /// scrolls. Throttled while scrolling unless `force` (e.g. when the reader
    /// closes). Marks the item finished at the end. Returns the full locator.
    pub fn record_progress(
        &self,
        item_id: &str,
        block_id: &str,
        offset: u32,
        at_end: bool,
        force: bool,
    ) -> Result<Option<Locator>> {
        let inner = &self.inner;
        let Some(doc) = inner.current_document(item_id)? else {
            return Ok(None);
        };
        // A PDF's position is its page; its text view must not replace it.
        if doc.version.raw_mime == PDF_MIME {
            return Ok(None);
        }
        let Some(mut locator) = locator_at(&doc.document, &doc.version.hash, block_id, offset)
        else {
            return Ok(None);
        };
        if at_end || doc.document.blocks.last().is_some_and(|b| b.id == block_id) {
            locator.progression = locator.progression.max(FINISHED_AT);
        }
        inner.record_locator(item_id, locator, force)
    }

    /// Records the page a PDF view shows (0-based). Reaching the last page
    /// of a multi-page document finishes it.
    pub fn record_page_progress(
        &self,
        item_id: &str,
        page: u32,
        page_count: u32,
        force: bool,
    ) -> Result<Option<Locator>> {
        let row = self
            .inner
            .store
            .item(item_id)?
            .ok_or(EngineError::NotFound)?;
        let (Some(content_hash), true) = (row.item.current_version, page_count > 0) else {
            return Ok(None);
        };
        let page = page.min(page_count - 1);
        let mut progression = page as f64 / page_count as f64;
        if page_count > 1 && page + 1 == page_count {
            progression = progression.max(FINISHED_AT);
        }
        let locator = Locator {
            content_hash,
            position: Position::Pdf {
                page,
                progression: 0.0,
            },
            quote: TextQuote::default(),
            progression,
        };
        self.inner.record_locator(item_id, locator, force)
    }
}

impl Inner {
    fn record_locator(
        &self,
        item_id: &str,
        locator: Locator,
        force: bool,
    ) -> Result<Option<Locator>> {
        let row = self.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let finishes = locator.progression >= FINISHED_AT && !row.item.finished;
        {
            let mut last = self.progress_throttle.lock();
            if let Some((when, progression)) = last.get(item_id)
                && !force
                && !finishes
                && when.elapsed() < PROGRESS_INTERVAL
                && (progression - locator.progression).abs() < 0.2
            {
                return Ok(Some(locator));
            }
            last.insert(item_id.to_string(), (Instant::now(), locator.progression));
        }
        self.record(
            Entity::Progress,
            item_id,
            fields::LOCATOR,
            serde_json::to_value(&locator)?,
            false,
        )?;
        self.store.meta_set(
            &format!("local_progress:{item_id}"),
            &serde_json::to_string(&locator)?,
        )?;
        if finishes {
            self.record(Entity::Item, item_id, fields::FINISHED, true.into(), false)?;
        }
        Ok(Some(locator))
    }
}

impl Engine {
    pub fn set_finished(&self, item_id: &str, finished: bool) -> Result<()> {
        self.inner.record(
            Entity::Item,
            item_id,
            fields::FINISHED,
            finished.into(),
            false,
        )
    }

    /// Back to unread and never started: not read (on FreshRSS too), not
    /// finished, no reading position, out of Continue Reading and recently
    /// read lists.
    pub fn mark_unread(&self, item_ids: &[String]) -> Result<()> {
        let inner = &self.inner;
        let mut upstream = false;
        for id in item_ids {
            let row = inner.store.item(id)?.ok_or(EngineError::NotFound)?;
            if row.item.read {
                let pending = row.item.upstream_id.is_some();
                upstream |= pending;
                inner.record(Entity::Item, id, fields::READ, false.into(), pending)?;
            }
            if row.item.finished {
                inner.record(Entity::Item, id, fields::FINISHED, false.into(), false)?;
            }
            if row.item.progress > 0.0 || row.item.last_read_at.is_some() {
                let hash = row.item.current_version.clone().unwrap_or_default();
                let locator = Locator::not_started(&hash);
                inner.progress_throttle.lock().remove(id);
                inner.record(
                    Entity::Progress,
                    id,
                    fields::LOCATOR,
                    serde_json::to_value(&locator)?,
                    false,
                )?;
                inner.store.meta_set(
                    &format!("local_progress:{id}"),
                    &serde_json::to_string(&locator)?,
                )?;
            }
        }
        if upstream {
            inner.push_in_background();
        }
        Ok(())
    }

    /// Where to resume reading, re-anchored to the current content version
    /// (the position may have been taken in the feed excerpt, or on another
    /// device).
    pub fn resume_point(&self, item_id: &str) -> Result<Option<ResumePoint>> {
        let inner = &self.inner;
        let Some(stored) = inner
            .store
            .progress(item_id)?
            .filter(|l| !l.is_not_started())
        else {
            return Ok(None);
        };
        let Some(doc) = inner.current_document(item_id)? else {
            return Ok(None);
        };
        let locator = remap_locator(&doc.document, &doc.version.hash, &stored);
        let (block_id, offset) = match &locator.position {
            Position::Block { block_id, offset } => (Some(block_id.clone()), *offset),
            _ => (None, 0),
        };
        let page = match &locator.position {
            Position::Pdf { page, .. } => Some(*page),
            _ => None,
        };
        let clock = inner
            .store
            .field_clock(Entity::Progress, item_id, fields::LOCATOR)?;
        let device = clock.as_ref().map(|h| h.node.clone());
        let from_other_device = device.as_deref().is_some_and(|d| d != inner.clock.node());
        let local: Option<Locator> = if from_other_device {
            inner
                .store
                .meta_get(&format!("local_progress:{item_id}"))?
                .and_then(|s| serde_json::from_str(&s).ok())
        } else {
            None
        };
        let local = local.map(|l| remap_locator(&doc.document, &doc.version.hash, &l));
        let (local_block_id, local_offset) = match local.map(|l| l.position) {
            Some(Position::Block { block_id, offset }) => (Some(block_id), offset),
            _ => (None, 0),
        };
        Ok(Some(ResumePoint {
            locator,
            block_id,
            offset,
            page,
            updated_at: clock.map(|h| h.wall_ms as i64).unwrap_or(0),
            from_other_device,
            device,
            local_block_id,
            local_offset,
        }))
    }

    /// Looks for a table of contents in a saved page.
    pub fn detect_series_in(&self, item_id: &str) -> Result<Option<SeriesProposal>> {
        let inner = &self.inner;
        let row = inner.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let Some(hash) = row.item.current_version else {
            return Ok(None);
        };
        let Some(version) = inner.store.content_version(&hash)? else {
            return Ok(None);
        };
        if version.raw_mime != PAGE_MIME {
            return Ok(None);
        }
        let Some(raw) = inner.store.get_blob(&version.raw_blob)? else {
            return Ok(None);
        };
        let url = version.source_url.or(row.item.url).unwrap_or_default();
        Ok(detect_series(&String::from_utf8_lossy(&raw), &url))
    }

    /// Fetches a table-of-contents page and proposes a series from it.
    pub async fn detect_series_at(&self, url: &str) -> Result<Option<SeriesProposal>> {
        let fetcher = self.inner.fetcher.read().clone();
        let page = fetcher
            .fetch(url, |_| true)
            .await
            .map_err(|e| EngineError::Network(e.to_string()))?;
        if let Some(proposal) = detect_series(&page.html, &page.final_url) {
            return Ok(Some(proposal));
        }
        // Many docs sites build their table of contents with JavaScript.
        if page.strategy != "webview"
            && let Some(rendered) = fetcher.render(url).await
        {
            return Ok(detect_series(&rendered.html, &rendered.final_url));
        }
        Ok(None)
    }

    /// Creates a series from confirmed chapters. Pages already saved join it;
    /// the others become chapters whose content is fetched when opened.
    pub fn create_series(
        &self,
        title: &str,
        toc_url: Option<&str>,
        chapters: &[ChapterLink],
    ) -> Result<String> {
        let inner = &self.inner;
        let series_id = ids::new_local_id();
        inner.record(
            Entity::Series,
            &series_id,
            fields::CREATE,
            serde_json::json!({ "title": title, "toc_url": toc_url }),
            false,
        )?;
        for (index, chapter) in chapters.iter().enumerate() {
            let url = &chapter.url;
            let item_id = match inner.saved_by_url(url)? {
                Some(row) => row.item.id,
                None => {
                    let id = ids::new_local_id();
                    let now = now_ms();
                    let item = Item {
                        id: id.clone(),
                        source_id: Some(ids::saved_source_id()),
                        account_id: None,
                        upstream_id: None,
                        kind: ItemKind::Chapter,
                        url: Some(url.clone()),
                        title: chapter.title.clone(),
                        author: None,
                        published_at: now,
                        added_at: now,
                        read: false,
                        starred: false,
                        series_id: None,
                        chapter_index: None,
                        chapter_level: 0,
                        current_version: None,
                        excerpt: None,
                        lang: None,
                        text_len: 0,
                        progress: 0.0,
                        finished: false,
                        last_read_at: None,
                    };
                    inner.record(
                        Entity::Item,
                        &id,
                        fields::CREATE,
                        serde_json::to_value(&item)?,
                        false,
                    )?;
                    id
                }
            };
            self.place_in_series(&series_id, &item_id, index as u32, chapter.level)?;
        }
        Ok(series_id)
    }

    pub fn add_to_series(&self, series_id: &str, item_id: &str, chapter_index: u32) -> Result<()> {
        self.place_in_series(series_id, item_id, chapter_index, 0)
    }

    fn place_in_series(
        &self,
        series_id: &str,
        item_id: &str,
        chapter_index: u32,
        chapter_level: u32,
    ) -> Result<()> {
        self.inner.record(
            Entity::Item,
            item_id,
            fields::SERIES,
            serde_json::json!({
                "series_id": series_id,
                "chapter_index": chapter_index,
                "chapter_level": chapter_level,
            }),
            false,
        )
    }

    pub fn remove_from_series(&self, item_id: &str) -> Result<()> {
        self.inner.record(
            Entity::Item,
            item_id,
            fields::SERIES,
            serde_json::json!({ "series_id": null, "chapter_index": null }),
            false,
        )
    }

    pub fn rename_series(&self, series_id: &str, title: &str) -> Result<()> {
        self.inner.record(
            Entity::Series,
            series_id,
            fields::TITLE,
            title.trim().into(),
            false,
        )
    }

    /// Deletes a series together with its chapters.
    pub fn delete_series(&self, series_id: &str) -> Result<()> {
        if self.inner.store.series(series_id)?.is_none() {
            return Err(EngineError::NotFound);
        }
        self.inner.delete_entry(series_id)
    }

    pub fn series_list(&self) -> Result<Vec<SeriesInfo>> {
        self.inner
            .store
            .series_list()?
            .into_iter()
            .map(|(s, _, _)| self.inner.series_info(s))
            .collect()
    }

    pub fn series_info(&self, series_id: &str) -> Result<Option<SeriesInfo>> {
        self.inner
            .store
            .series(series_id)?
            .map(|s| self.inner.series_info(s))
            .transpose()
    }
}
