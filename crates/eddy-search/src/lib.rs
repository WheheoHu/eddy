//! Full-text search over items with Chinese word segmentation (jieba) and
//! lowercase matching for Latin scripts. Clients index their local data; the
//! server indexes the whole history.

use std::path::Path;

use parking_lot::Mutex;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

const TOKENIZER: &str = "eddy_jieba";
const MAX_BODY_CHARS: usize = 60_000;
const WRITER_HEAP: usize = 30_000_000;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("搜索索引出错：{0}")]
    Index(#[from] tantivy::TantivyError),
    #[error("搜索索引目录出错：{0}")]
    Dir(String),
}

pub type Result<T> = std::result::Result<T, SearchError>;

#[derive(Clone, Debug, PartialEq)]
pub struct IndexDoc {
    pub item_id: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub published_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub item_id: String,
    pub score: f32,
    /// Excerpt around the best match.
    pub snippet: String,
    /// Highlighted ranges in `snippet`, UTF-16 offsets.
    pub highlights: Vec<(u32, u32)>,
}

struct Fields {
    item_id: Field,
    title: Field,
    body: Field,
    source: Field,
    published_at: Field,
}

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: Fields,
    dirty: Mutex<bool>,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let text = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let fields = Fields {
        item_id: b.add_text_field("item_id", STRING | STORED),
        title: b.add_text_field("title", text.clone()),
        body: b.add_text_field("body", text.clone()),
        source: b.add_text_field("source", text),
        published_at: b.add_i64_field("published_at", STORED),
    };
    (b.build(), fields)
}

impl SearchIndex {
    /// Opens or creates the index in `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| SearchError::Dir(e.to_string()))?;
        let (schema, fields) = schema();
        let directory = tantivy::directory::MmapDirectory::open(dir)
            .map_err(|e| SearchError::Dir(e.to_string()))?;
        let index = Index::open_or_create(directory, schema)?;
        Self::with_index(index, fields)
    }

    pub fn in_memory() -> Result<Self> {
        let (schema, fields) = schema();
        Self::with_index(Index::create_in_ram(schema), fields)
    }

    fn with_index(index: Index, fields: Fields) -> Result<Self> {
        let analyzer = TextAnalyzer::builder(tantivy_jieba::JiebaTokenizer::new())
            .filter(LowerCaser)
            .build();
        index.tokenizers().register(TOKENIZER, analyzer);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let writer = index.writer(WRITER_HEAP)?;
        Ok(SearchIndex {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
            dirty: Mutex::new(false),
        })
    }

    /// Adds or replaces an item. Visible to searches after the next commit
    /// (searches commit pending changes first).
    pub fn upsert(&self, d: &IndexDoc) -> Result<()> {
        let f = &self.fields;
        let body: String = d.body.chars().take(MAX_BODY_CHARS).collect();
        let writer = self.writer.lock();
        writer.delete_term(Term::from_field_text(f.item_id, &d.item_id));
        writer.add_document(doc!(
            f.item_id => d.item_id.clone(),
            f.title => d.title.clone(),
            f.body => body,
            f.source => d.source.clone(),
            f.published_at => d.published_at,
        ))?;
        *self.dirty.lock() = true;
        Ok(())
    }

    pub fn delete(&self, item_id: &str) -> Result<()> {
        self.writer
            .lock()
            .delete_term(Term::from_field_text(self.fields.item_id, item_id));
        *self.dirty.lock() = true;
        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut dirty = self.dirty.lock();
        if *dirty {
            self.writer.lock().commit()?;
            self.reader.reload()?;
            *dirty = false;
        }
        Ok(())
    }

    pub fn len(&self) -> Result<u64> {
        self.commit()?;
        Ok(self.reader.searcher().num_docs())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn clear(&self) -> Result<()> {
        self.writer.lock().delete_all_documents()?;
        *self.dirty.lock() = true;
        self.commit()
    }

    /// Ranked matches for free text. Query syntax errors never fail: the
    /// parser falls back to plain terms.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        self.commit()?;
        let query_text = query.trim();
        if query_text.is_empty() {
            return Ok(Vec::new());
        }
        let f = &self.fields;
        let mut parser = QueryParser::for_index(&self.index, vec![f.title, f.body, f.source]);
        parser.set_field_boost(f.title, 3.0);
        parser.set_conjunction_by_default();
        let (query, _) = parser.parse_query_lenient(query_text);
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut snippets = SnippetGenerator::create(&searcher, &*query, f.body)?;
        snippets.set_max_num_chars(160);
        let mut hits = Vec::new();
        for (score, address) in top {
            let doc: TantivyDocument = searcher.doc(address)?;
            let Some(item_id) = doc
                .get_first(f.item_id)
                .and_then(|v| v.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            let snippet = snippets.snippet_from_doc(&doc);
            let fragment = snippet.fragment().to_string();
            let mut ranges: Vec<(usize, usize)> = snippet
                .highlighted()
                .iter()
                .map(|r| (r.start, r.end))
                .collect();
            ranges.sort_unstable();
            // Search-mode segmentation yields overlapping sub-words; merge them.
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (s, e) in ranges {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            let highlights = merged
                .into_iter()
                .map(|(s, e)| (utf16_at(&fragment, s), utf16_at(&fragment, e)))
                .collect();
            hits.push(Hit {
                item_id,
                score,
                snippet: fragment,
                highlights,
            });
        }
        Ok(hits)
    }
}

fn utf16_at(s: &str, byte: usize) -> u32 {
    eddy_model::utf16_len(&s[..byte.min(s.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, title: &str, body: &str) -> IndexDoc {
        IndexDoc {
            item_id: id.into(),
            title: title.into(),
            body: body.into(),
            source: "Blog".into(),
            published_at: 0,
        }
    }

    #[test]
    fn chinese_and_english_search_with_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let index = SearchIndex::open(dir.path()).unwrap();
        index
            .upsert(&doc(
                "a",
                "路径追踪入门",
                "路径追踪是一种基于蒙特卡洛积分的渲染算法，通过重要性采样降低方差。",
            ))
            .unwrap();
        index
            .upsert(&doc(
                "b",
                "Rust ownership",
                "Ownership and borrowing in Rust explained with examples.",
            ))
            .unwrap();
        index
            .upsert(&doc("c", "Cooking", "A recipe for dumplings."))
            .unwrap();

        let hits = index.search("蒙特卡洛", 10).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.item_id.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        assert!(hits[0].snippet.contains("蒙特卡洛"));
        let (s, e) = hits[0].highlights[0];
        let units: Vec<u16> = hits[0].snippet.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[s as usize..e as usize]).unwrap(),
            "蒙特卡洛"
        );

        assert_eq!(index.search("渲染 算法", 10).unwrap()[0].item_id, "a");
        assert_eq!(index.search("BORROWING", 10).unwrap()[0].item_id, "b");
        assert!(index.search("\"unbalanced (", 10).is_ok());
        assert!(index.search("nothing-matches-this", 10).unwrap().is_empty());

        // Upsert replaces, delete removes, data survives reopening.
        index
            .upsert(&doc("c", "Cooking", "A recipe for noodles."))
            .unwrap();
        assert!(index.search("dumplings", 10).unwrap().is_empty());
        index.delete("b").unwrap();
        assert_eq!(index.len().unwrap(), 2);
        drop(index);
        let index = SearchIndex::open(dir.path()).unwrap();
        assert_eq!(index.search("noodles", 10).unwrap()[0].item_id, "c");
    }

    #[test]
    fn title_matches_rank_above_body_matches() {
        let index = SearchIndex::in_memory().unwrap();
        index
            .upsert(&doc(
                "body",
                "Notes",
                "a long article that mentions denoising once",
            ))
            .unwrap();
        index
            .upsert(&doc("title", "Denoising", "about filters"))
            .unwrap();
        let hits = index.search("denoising", 10).unwrap();
        assert_eq!(hits[0].item_id, "title");
    }
}
