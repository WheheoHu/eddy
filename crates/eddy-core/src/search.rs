use eddy_search::IndexDoc;
use eddy_store::{ItemFilter, ItemQuery, ItemRow};

use crate::content::Ingested;
use crate::engine::{Engine, Inner};
use crate::{LogLevel, Result};

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub item: ItemRow,
    pub snippet: String,
    /// Highlighted ranges in `snippet`, UTF-16 offsets.
    pub highlights: Vec<(u32, u32)>,
}

impl Inner {
    pub(crate) fn index_item(&self, item_id: &str, ingested: &Ingested) {
        let Some(index) = &self.search else { return };
        let Ok(Some(row)) = self.store.item(item_id) else {
            return;
        };
        let doc = IndexDoc {
            item_id: item_id.to_string(),
            title: row.item.title.clone(),
            body: ingested.document.plain_text(),
            source: row.source_title.unwrap_or_default(),
            published_at: row.item.published_at,
        };
        if let Err(e) = index.upsert(&doc) {
            self.platform
                .log(LogLevel::Warn, &format!("indexing failed: {e}"));
        }
    }

    pub(crate) fn unindex_item(&self, item_id: &str) {
        if let Some(index) = &self.search
            && let Err(e) = index.delete(item_id)
        {
            self.platform
                .log(LogLevel::Warn, &format!("unindexing failed: {e}"));
        }
    }

    pub(crate) fn commit_index(&self) {
        if let Some(index) = &self.search
            && let Err(e) = index.commit()
        {
            self.platform
                .log(LogLevel::Warn, &format!("index commit failed: {e}"));
        }
    }

    /// Re-indexes every item with content. Returns the number indexed.
    pub(crate) fn rebuild_index(&self) -> Result<u32> {
        let Some(index) = &self.search else {
            return Ok(0);
        };
        index
            .clear()
            .map_err(|e| crate::EngineError::Invalid(e.to_string()))?;
        let mut count = 0;
        let mut offset = 0;
        loop {
            let rows = self.store.items(&ItemQuery {
                filter: ItemFilter::All,
                unread_only: false,
                limit: 500,
                offset,
            })?;
            let chapters = if offset == 0 {
                self.chapter_rows()?
            } else {
                Vec::new()
            };
            if rows.is_empty() && chapters.is_empty() {
                break;
            }
            offset += rows.len() as u32;
            for row in rows.into_iter().chain(chapters) {
                if let Ok(Some(doc)) = self.current_document(&row.item.id) {
                    let _ = index.upsert(&IndexDoc {
                        item_id: row.item.id.clone(),
                        title: row.item.title.clone(),
                        body: doc.document.plain_text(),
                        source: row.source_title.clone().unwrap_or_default(),
                        published_at: row.item.published_at,
                    });
                    count += 1;
                }
            }
        }
        self.commit_index();
        Ok(count)
    }

    fn chapter_rows(&self) -> Result<Vec<ItemRow>> {
        let mut out = Vec::new();
        for (series, _, _) in self.store.series_list()? {
            out.extend(self.store.items(&ItemQuery {
                filter: ItemFilter::Series(series.id),
                unread_only: false,
                limit: 10_000,
                offset: 0,
            })?);
        }
        Ok(out)
    }
}

impl Engine {
    /// Searches local items (titles, full text, source names), best first.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchResult>> {
        let inner = &self.inner;
        let Some(index) = &inner.search else {
            return Ok(Vec::new());
        };
        if index.is_empty().unwrap_or(true) {
            inner.rebuild_index()?;
        }
        let hits = index
            .search(query, limit as usize)
            .map_err(|e| crate::EngineError::Invalid(e.to_string()))?;
        let ids: Vec<String> = hits.iter().map(|h| h.item_id.clone()).collect();
        let rows = inner.store.items(&ItemQuery {
            filter: ItemFilter::Ids(ids),
            unread_only: false,
            limit: limit.max(1),
            offset: 0,
        })?;
        Ok(hits
            .into_iter()
            .filter_map(|hit| {
                let item = rows.iter().find(|r| r.item.id == hit.item_id)?.clone();
                Some(SearchResult {
                    item,
                    snippet: hit.snippet,
                    highlights: hit.highlights,
                })
            })
            .collect())
    }

    pub fn rebuild_search_index(&self) -> Result<u32> {
        self.inner.rebuild_index()
    }
}
