//! Importing local files. An EPUB becomes a series whose chapters read in
//! the reflowed reader; a PDF becomes a saved document shown by the
//! platform's PDF view, with its text (reflowed by `pdf_layout`) kept for
//! search, summaries and the text view. The original file is every
//! version's pinned raw source.

use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eddy_doc::{ConvertOptions, html_to_document};
use eddy_epub::{Book, Epub, EpubError};
use eddy_model::{
    BlockKind, ContentVersion, Document, EPUB_MIME, EPUB_SCHEME, Entity, Item, ItemKind, PDF_MIME, PDF_SCHEME,
    fields, ids, now_ms,
};
use serde_json::json;

use crate::engine::{Engine, Inner};
use crate::{EngineError, Result};

/// Largest file accepted: it has to fit a sync server upload too.
pub const MAX_IMPORT_BYTES: u64 = eddy_sync::MAX_BLOB_BYTES as u64;

/// Text of a PDF with the geometry the layout rules need: each page's
/// lines with their boxes and font sizes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfText {
    pub title: Option<String>,
    pub author: Option<String>,
    /// Pages in order.
    pub pages: Vec<PdfPage>,
}

/// One page of text. Coordinates are PDF points with the origin at the
/// bottom-left corner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfPage {
    pub width: f32,
    pub height: f32,
    /// Lines in content-stream order, not reading order.
    pub lines: Vec<PdfLine>,
    /// Raster images placed on the page, in content-stream order.
    pub images: Vec<PdfImage>,
    /// Large rectangles drawn on the page (panels, the frame of a slide on
    /// a notes page). Not pictures by themselves, but their area can be
    /// shown as one, addressed by the rectangle's object index.
    pub frames: Vec<PdfImage>,
}

/// A raster image on a page. Its pixels are read on demand through
/// `PdfReader::image`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfImage {
    /// Position among the page's objects, stable for a given file.
    pub index: u32,
    /// Placement on the page, in the same coordinates as lines.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Identifies the same image data drawn on several pages (a logo, a
    /// watermark).
    pub fingerprint: u64,
}

/// One visual line of text with its bounding box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfLine {
    pub text: String,
    /// Left edge of the box.
    pub x: f32,
    /// Bottom edge of the box.
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Font size most of the line is set in.
    pub font_size: f32,
    pub bold: bool,
}

/// Reads PDF text. The app and CLI use `eddy_pdf`; without one (servers),
/// PDFs import and open but have no text to search, summarize or reflow.
pub trait PdfReader: Send + Sync {
    fn read(&self, path: &Path) -> Option<PdfText>;

    /// Encoded bytes and MIME type of image `index` on page `page`.
    fn image(&self, _path: &Path, _page: u32, _index: u32) -> Option<(Vec<u8>, &'static str)> {
        None
    }

    /// Several images, `(page, index)` each, in request order. Readers
    /// that can share work between images on a page override this.
    fn images(&self, path: &Path, requests: &[(u32, u32)]) -> Vec<Option<(Vec<u8>, &'static str)>> {
        requests
            .iter()
            .map(|&(page, index)| self.image(path, page, index))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// What to open: the PDF, or the book's first chapter.
    pub item_id: String,
    pub series_id: Option<String>,
    pub title: String,
    /// The file had been imported before; nothing new was created.
    pub existing: bool,
}

impl Engine {
    pub fn set_pdf_reader(&self, reader: Option<Arc<dyn PdfReader>>) {
        *self.inner.pdf_reader.write() = reader;
    }

    /// Imports an EPUB or PDF file. The same file imported again returns
    /// what the first import created.
    pub async fn import_file(&self, path: &Path) -> Result<Imported> {
        let inner = self.inner.clone();
        let path = path.to_path_buf();
        let result = tokio::task::spawn_blocking(move || inner.import_file(&path))
            .await
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        self.inner.commit_index();
        result
    }

    /// Local path of an item's original file (an imported PDF), downloaded
    /// from the sync server when this device does not have it yet.
    pub async fn content_file(&self, item_id: &str) -> Result<Option<PathBuf>> {
        let inner = &self.inner;
        let row = inner.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let Some(version) = row
            .item
            .current_version
            .as_deref()
            .map(|h| inner.store.content_version(h))
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        inner.local_blob_file(&version.raw_blob).await
    }

    /// The chapter a link inside a book points to.
    pub fn item_for_link(&self, url: &str) -> Result<Option<String>> {
        let store = &self.inner.store;
        let Some((key, part)) = parse_archive_url(url) else {
            return Ok(None);
        };
        let target = archive_url(&key, &part);
        if let Some(id) = store.item_by_source_url(&target)? {
            return Ok(Some(id));
        }
        // Later files of a chapter are found through its first one.
        let Some(book) = self.inner.open_book(&key)?.map(|(_, book)| book) else {
            return Ok(None);
        };
        let Some(chapter) = book.chapters.iter().find(|c| c.parts.contains(&part)) else {
            return Ok(None);
        };
        Ok(store.item_by_source_url(&archive_url(&key, &chapter.parts[0]))?)
    }
}

impl Inner {
    fn import_file(self: &Arc<Self>, path: &Path) -> Result<Imported> {
        let size = std::fs::metadata(path).map_err(read_error)?.len();
        if size > MAX_IMPORT_BYTES {
            return Err(EngineError::Invalid(format!(
                "文件太大，最多支持 {} MB。",
                MAX_IMPORT_BYTES / 1024 / 1024
            )));
        }
        let bytes = std::fs::read(path).map_err(read_error)?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().trim().to_string())
            .unwrap_or_default();
        let key = ids::sha256_hex(&bytes);
        if let Some(existing) = self.existing_import(&key)? {
            return Ok(existing);
        }
        if is_pdf(&bytes) {
            self.import_pdf(&bytes, &key, &name)
        } else if bytes.starts_with(b"PK\x03\x04") {
            self.import_epub(&bytes, &key, &name)
        } else {
            Err(EngineError::Invalid("只支持 EPUB 和 PDF 文件。".into()))
        }
    }

    fn existing_import(&self, key: &str) -> Result<Option<Imported>> {
        for item_id in self.store.items_with_raw_blob(key)? {
            let Some(row) = self.store.item(&item_id)? else {
                continue;
            };
            if let Some(series_id) = row.item.series_id {
                if let Some(series) = self.store.series(&series_id)? {
                    return Ok(Some(Imported {
                        item_id,
                        series_id: Some(series_id),
                        title: series.title,
                        existing: true,
                    }));
                }
            } else if row.item.kind == ItemKind::Saved {
                return Ok(Some(Imported {
                    item_id,
                    series_id: None,
                    title: row.item.title,
                    existing: true,
                }));
            }
        }
        Ok(None)
    }

    fn import_epub(&self, bytes: &[u8], key: &str, file_name: &str) -> Result<Imported> {
        let mut epub = Epub::new(Cursor::new(bytes)).map_err(epub_error)?;
        let book = epub.book().map_err(epub_error)?;
        let book_title = book.title.clone().unwrap_or_else(|| file_name.to_string());
        let mut chapters = Vec::new();
        for chapter in &book.chapters {
            let base = archive_url(key, &chapter.parts[0]);
            let html = epub.chapter_html(chapter);
            let (title, document) = chapter_document(&html, &base, chapter.title.as_deref());
            if !document.blocks.is_empty() {
                chapters.push((title.unwrap_or_else(|| book_title.clone()), base, document));
            }
        }
        if chapters.is_empty() {
            return Err(EngineError::Invalid(
                "这本 EPUB 里没有可阅读的内容。".into(),
            ));
        }

        self.store.put_blob_keyed(key, bytes, EPUB_MIME, true)?;
        let series_id = ids::new_local_id();
        self.record(
            Entity::Series,
            &series_id,
            fields::CREATE,
            json!({
                "title": book_title,
                "toc_url": null,
                "author": book.author,
                "cover": book.cover.as_deref().map(|p| archive_url(key, p)),
            }),
            false,
        )?;
        let mut first = None;
        for (index, (title, base, document)) in chapters.into_iter().enumerate() {
            let item_id =
                self.create_imported_item(ItemKind::Chapter, &title, book.author.as_deref())?;
            let ingested = self.ingest_document(&item_id, document, key, EPUB_MIME, Some(&base))?;
            self.switch_version(&item_id, &ingested)?;
            self.record(
                Entity::Item,
                &item_id,
                fields::SERIES,
                json!({ "series_id": series_id, "chapter_index": index }),
                false,
            )?;
            self.after_ingest(&item_id, &ingested);
            first.get_or_insert(item_id);
        }
        Ok(Imported {
            item_id: first.expect("at least one chapter"),
            series_id: Some(series_id),
            title: book_title,
            existing: false,
        })
    }

    fn import_pdf(self: &Arc<Self>, bytes: &[u8], key: &str, file_name: &str) -> Result<Imported> {
        self.store.put_blob_keyed(key, bytes, PDF_MIME, true)?;
        let text = self.read_pdf(key)?.unwrap_or_default();
        let title = pdf_title(text.title.as_deref(), file_name);
        let item_id = self.create_imported_item(ItemKind::Saved, &title, text.author.as_deref())?;
        let document = crate::pdf_layout::document(&text.pages, key);
        let ingested = self.ingest_document(&item_id, document, key, PDF_MIME, None)?;
        self.switch_version(&item_id, &ingested)?;
        self.after_ingest(&item_id, &ingested);
        self.warm_pdf_images(&ingested.document);
        Ok(Imported {
            item_id,
            series_id: None,
            title,
            existing: false,
        })
    }

    fn create_imported_item(
        &self,
        kind: ItemKind,
        title: &str,
        author: Option<&str>,
    ) -> Result<String> {
        let id = ids::new_local_id();
        let now = now_ms();
        let item = Item {
            id: id.clone(),
            source_id: Some(ids::saved_source_id()),
            account_id: None,
            upstream_id: None,
            kind,
            url: None,
            title: title.to_string(),
            author: author.map(str::to_string),
            published_at: now,
            added_at: now,
            read: false,
            starred: false,
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
        self.record(
            Entity::Item,
            &id,
            fields::CREATE,
            serde_json::to_value(&item)?,
            false,
        )?;
        Ok(id)
    }

    fn read_pdf(&self, key: &str) -> Result<Option<PdfText>> {
        let reader = self.pdf_reader.read().clone();
        let (Some(reader), Some(path)) = (reader, self.store.blob_file(key)?) else {
            return Ok(None);
        };
        Ok(reader.read(&path))
    }

    /// The text document of a stored PDF, when a PDF reader is available.
    pub(crate) fn pdf_document(&self, key: &str) -> Result<Option<Document>> {
        Ok(self.read_pdf(key)?.map(|t| crate::pdf_layout::document(&t.pages, key)))
    }

    /// Regenerates an EPUB chapter's document from the stored book.
    pub(crate) fn epub_chapter_document(
        &self,
        version: &ContentVersion,
        title: Option<&str>,
    ) -> Result<Option<Document>> {
        let Some(url) = version.source_url.as_deref() else {
            return Ok(None);
        };
        let Some((_, part)) = parse_archive_url(url) else {
            return Ok(None);
        };
        let Some((mut epub, book)) = self.open_book(&version.raw_blob)? else {
            return Ok(None);
        };
        let Some(chapter) = book
            .chapters
            .iter()
            .find(|c| c.parts.first() == Some(&part))
        else {
            return Ok(None);
        };
        let html = epub.chapter_html(chapter);
        Ok(Some(html_to_document(
            &html,
            &ConvertOptions {
                base_url: Some(url),
                title,
            },
        )))
    }

    #[allow(clippy::type_complexity)]
    fn open_book(&self, key: &str) -> Result<Option<(Epub<BufReader<std::fs::File>>, Book)>> {
        let Some(path) = self.store.blob_file(key)? else {
            return Ok(None);
        };
        let file = std::fs::File::open(path).map_err(read_error)?;
        let mut epub = Epub::new(BufReader::new(file)).map_err(epub_error)?;
        let book = epub.book().map_err(epub_error)?;
        Ok(Some((epub, book)))
    }

    /// A file inside a stored book (an image).
    pub(crate) async fn archive_file(&self, url: &str) -> Result<Option<(Vec<u8>, &'static str)>> {
        let Some((key, path)) = parse_archive_url(url) else {
            return Ok(None);
        };
        let Some(file) = self.local_blob_file(&key).await? else {
            return Ok(None);
        };
        let mime = eddy_epub::mime_for(&path);
        let bytes = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(file).ok()?;
            Epub::new(BufReader::new(file)).ok()?.read(&path)
        })
        .await
        .ok()
        .flatten();
        Ok(bytes.map(|b| (b, mime)))
    }

    /// An image inside a stored PDF, decoded by the PDF reader.
    pub(crate) async fn pdf_image(&self, url: &str) -> Result<Option<(Vec<u8>, &'static str)>> {
        let Some((key, page, index)) = parse_pdf_image_url(url) else {
            return Ok(None);
        };
        let reader = self.pdf_reader.read().clone();
        let Some(reader) = reader else {
            return Ok(None);
        };
        let Some(file) = self.local_blob_file(&key).await? else {
            return Ok(None);
        };
        Ok(
            tokio::task::spawn_blocking(move || reader.image(&file, page, index))
                .await
                .ok()
                .flatten(),
        )
    }

    /// Path of a blob, downloading it from the sync server when missing here.
    pub(crate) async fn local_blob_file(&self, key: &str) -> Result<Option<PathBuf>> {
        if let Some(path) = self.store.blob_file(key)? {
            return Ok(Some(path));
        }
        let upstream = self.upstream.read().clone();
        let Some(upstream) = upstream else {
            return Ok(None);
        };
        let Some((bytes, mime)) = upstream.fetch_blob(key).await? else {
            return Ok(None);
        };
        if ids::sha256_hex(&bytes) != key {
            return Err(EngineError::Network("下载的文件不完整，请重试。".into()));
        }
        self.store.put_blob_keyed(key, &bytes, &mime, true)?;
        Ok(self.store.blob_file(key)?)
    }
}

/// `epub://<book blob>/<path>` for a file inside an imported book.
pub(crate) fn archive_url(key: &str, path: &str) -> String {
    let raw = format!("{EPUB_SCHEME}://{key}/{}", eddy_epub::encode_path(path));
    url::Url::parse(&raw).map_or(raw, |u| u.to_string())
}

/// The book blob and archive path of an `epub://` URL, fragment dropped.
pub(crate) fn parse_archive_url(url: &str) -> Option<(String, String)> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != EPUB_SCHEME {
        return None;
    }
    let key = url.host_str()?.to_string();
    let path = eddy_epub::decode_path(url.path());
    (!path.is_empty()).then_some((key, path))
}

/// The PDF blob, page and object index of a `pdf://` image URL.
pub(crate) fn parse_pdf_image_url(url: &str) -> Option<(String, u32, u32)> {
    let url = url::Url::parse(url).ok()?;
    if url.scheme() != PDF_SCHEME {
        return None;
    }
    let key = url.host_str()?.to_string();
    let mut parts = url.path().trim_start_matches('/').split('/');
    let page = parts.next()?.parse().ok()?;
    let index = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((key, page, index))
}

/// Converts a chapter. Untitled chapters take their opening heading as the
/// title (dropping it from the text, as for titled ones), else their first
/// words.
fn chapter_document(html: &str, base: &str, title: Option<&str>) -> (Option<String>, Document) {
    let convert = |title: Option<&str>| {
        html_to_document(
            html,
            &ConvertOptions {
                base_url: Some(base),
                title,
            },
        )
    };
    if let Some(title) = title {
        return (Some(title.to_string()), convert(Some(title)));
    }
    let document = convert(None);
    let heading = document
        .blocks
        .iter()
        .take(3)
        .find(|b| matches!(b.kind, BlockKind::Heading { .. }))
        .map(|b| b.plain_text().trim().to_string())
        .filter(|t| !t.is_empty());
    if let Some(heading) = heading {
        let document = convert(Some(&heading));
        return (Some(heading), document);
    }
    let excerpt = document.excerpt(48);
    ((!excerpt.is_empty()).then_some(excerpt), document)
}

fn is_pdf(bytes: &[u8]) -> bool {
    // The header may follow a little junk.
    bytes[..bytes.len().min(1024)]
        .windows(5)
        .any(|w| w == b"%PDF-")
}

/// The metadata title unless it is a leftover from the authoring tool.
fn pdf_title(meta: Option<&str>, file_name: &str) -> String {
    let junk = [
        "untitled",
        "microsoft word",
        ".doc",
        ".pdf",
        ".tex",
        ".indd",
        "slide 1",
    ];
    meta.map(str::trim)
        .filter(|t| {
            let lower = t.to_lowercase();
            t.chars().count() >= 3 && !junk.iter().any(|j| lower.contains(j))
        })
        .map(str::to_string)
        .or_else(|| (!file_name.is_empty()).then(|| file_name.to_string()))
        .unwrap_or_else(|| "PDF".into())
}

fn epub_error(e: EpubError) -> EngineError {
    EngineError::Invalid(e.to_string())
}

fn read_error(e: std::io::Error) -> EngineError {
    EngineError::Invalid(format!("无法读取文件：{e}"))
}
