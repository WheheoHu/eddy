use eddy_doc::{ConvertOptions, SCHEMA_VERSION, html_to_document};
use eddy_model::{
    ContentOrigin, ContentVersion, Document, EPUB_MIME, Entity, PDF_MIME, fields, ids::sha256_hex,
    now_ms,
};

use crate::engine::Inner;
use crate::{EngineError, Result};

/// A content version together with its (possibly regenerated) document.
#[derive(Clone, Debug)]
pub struct LoadedDocument {
    pub version: ContentVersion,
    pub document: Document,
}

pub(crate) struct Ingested {
    pub version: ContentVersion,
    pub document: Document,
    pub excerpt: String,
    pub lang: Option<String>,
}

impl Ingested {
    /// Derived list metadata, synced as the item's `meta` op.
    pub(crate) fn meta_value(&self) -> serde_json::Value {
        serde_json::json!({ "excerpt": self.excerpt, "text_len": self.version.text_len, "lang": self.lang })
    }
}

impl Inner {
    /// Stores raw HTML as a new content version of `item_id` and converts it.
    /// Raw content of saved pages is pinned: it cannot be fetched again.
    pub(crate) fn ingest_html(
        &self,
        item_id: &str,
        html: &str,
        origin: ContentOrigin,
        source_url: Option<&str>,
        title: Option<&str>,
        pin_raw: bool,
    ) -> Result<Ingested> {
        let raw_blob = self.store.put_blob(html.as_bytes(), "text/html", pin_raw)?;
        let document = html_to_document(
            html,
            &ConvertOptions {
                base_url: source_url,
                title,
            },
        );
        let doc_blob =
            self.store
                .put_blob(&serde_json::to_vec(&document)?, "application/json", false)?;
        let hash =
            sha256_hex(format!("{}\u{0}{}\u{0}{}", item_id, origin.as_str(), raw_blob).as_bytes());
        let text_len = document.text_len() as u32;
        let version = ContentVersion {
            hash,
            item_id: item_id.to_string(),
            origin,
            source_url: source_url.map(str::to_string),
            raw_blob,
            raw_mime: "text/html".into(),
            doc_blob: Some(doc_blob),
            doc_schema: Some(SCHEMA_VERSION),
            text_len,
            created_at: now_ms(),
        };
        self.store.insert_content_version(&version)?;
        let excerpt = document.excerpt(240);
        let lang = detect_lang(&document);
        Ok(Ingested {
            version,
            document,
            excerpt,
            lang,
        })
    }

    /// Stores a document made from a raw file that is already stored (an
    /// import) as a new content version. It syncs: other devices cannot
    /// fetch the file from anywhere else.
    pub(crate) fn ingest_document(
        &self,
        item_id: &str,
        document: Document,
        raw_blob: &str,
        raw_mime: &str,
        source_url: Option<&str>,
    ) -> Result<Ingested> {
        let doc_blob =
            self.store
                .put_blob(&serde_json::to_vec(&document)?, "application/json", false)?;
        let origin = ContentOrigin::Imported;
        let hash =
            sha256_hex(format!("{}\u{0}{}\u{0}{}", item_id, origin.as_str(), raw_blob).as_bytes());
        let version = ContentVersion {
            hash,
            item_id: item_id.to_string(),
            origin,
            source_url: source_url.map(str::to_string),
            raw_blob: raw_blob.to_string(),
            raw_mime: raw_mime.to_string(),
            doc_blob: Some(doc_blob),
            doc_schema: Some(SCHEMA_VERSION),
            text_len: document.text_len() as u32,
            created_at: now_ms(),
        };
        self.store.insert_content_version(&version)?;
        self.record(
            Entity::ContentVersion,
            &version.hash,
            fields::CREATE,
            serde_json::to_value(&version)?,
            false,
        )?;
        let excerpt = document.excerpt(240);
        let lang = detect_lang(&document);
        Ok(Ingested {
            version,
            document,
            excerpt,
            lang,
        })
    }

    /// Loads the document of a content version, regenerating it from the raw
    /// source when it was evicted or produced by an older schema.
    pub(crate) fn load_version(
        &self,
        version: &ContentVersion,
        title: Option<&str>,
    ) -> Result<Option<Document>> {
        if version.doc_schema == Some(SCHEMA_VERSION)
            && let Some(blob) = &version.doc_blob
            && let Some(bytes) = self.store.get_blob(blob)?
            && let Ok(doc) = serde_json::from_slice::<Document>(&bytes)
        {
            return Ok(Some(doc));
        }
        let document = match version.raw_mime.as_str() {
            EPUB_MIME => match self.epub_chapter_document(version, title)? {
                Some(doc) => doc,
                None => return Ok(None),
            },
            // Without a PDF reader (servers, tools) the file still opens,
            // just without text; that is not worth caching.
            PDF_MIME => match self.pdf_document(&version.raw_blob)? {
                Some(doc) => doc,
                None => {
                    return Ok(Some(Document {
                        schema_version: SCHEMA_VERSION,
                        blocks: Vec::new(),
                    }));
                }
            },
            _ => {
                let Some(raw) = self.store.get_blob(&version.raw_blob)? else {
                    return Ok(None);
                };
                let html = String::from_utf8_lossy(&raw);
                if version.raw_mime == crate::fulltext::PAGE_MIME {
                    match self.regenerate_page(version, &html) {
                        Some(doc) => doc,
                        None => return Ok(None),
                    }
                } else {
                    html_to_document(
                        &html,
                        &ConvertOptions {
                            base_url: version.source_url.as_deref(),
                            title,
                        },
                    )
                }
            }
        };
        let doc_blob =
            self.store
                .put_blob(&serde_json::to_vec(&document)?, "application/json", false)?;
        self.store.set_doc_blob(
            &version.hash,
            &doc_blob,
            SCHEMA_VERSION,
            document.text_len() as u32,
        )?;
        Ok(Some(document))
    }

    pub(crate) fn current_document(&self, item_id: &str) -> Result<Option<LoadedDocument>> {
        let row = self.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let Some(hash) = row.item.current_version.as_deref() else {
            return Ok(None);
        };
        let Some(version) = self.store.content_version(hash)? else {
            return Ok(None);
        };
        let document = self.load_version(&version, Some(&row.item.title))?;
        Ok(document.map(|document| LoadedDocument { version, document }))
    }
}

/// ISO 639-3 code of the dominant language, when confidently detected.
pub(crate) fn detect_lang(doc: &Document) -> Option<String> {
    let text: String = doc.plain_text().chars().take(4000).collect();
    let info = whatlang::detect(&text)?;
    info.is_reliable().then(|| info.lang().code().to_string())
}
