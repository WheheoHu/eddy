use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use eddy_ai::Summary;
use eddy_model::{BlockKind, ContentFormat, Document, EPUB_MIME, ItemKind, PDF_MIME};
use eddy_store::ItemRow;

use crate::engine::{Engine, Inner};
use crate::epub_export::{EmbeddedImage, EpubPackageInput, build_epub_archive};
use crate::{BatchCallback, EngineError, LogLevel, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf,
    Epub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportContentMode {
    Original,
    Bilingual,
    Translation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportScope {
    Whole,
    CurrentChapter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    pub exportable: bool,
    pub series_id: Option<String>,
    pub chapter_item_id: Option<String>,
    pub series_title: Option<String>,
    pub item_title: String,
    pub whole_filename: String,
    pub chapter_filename: Option<String>,
    pub can_choose_scope: bool,
    pub is_book: bool,
    pub chapter_count: u32,
    pub has_cached_summary_whole: bool,
    pub has_cached_summary_chapter: bool,
    pub can_passthrough_epub: bool,
    pub can_passthrough_pdf: bool,
}

impl ExportOptions {
    fn ineligible(title: String) -> Self {
        ExportOptions {
            exportable: false,
            series_id: None,
            chapter_item_id: None,
            series_title: None,
            whole_filename: sanitize_filename(&title),
            item_title: title,
            chapter_filename: None,
            can_choose_scope: false,
            is_book: false,
            chapter_count: 0,
            has_cached_summary_whole: false,
            has_cached_summary_chapter: false,
            can_passthrough_epub: false,
            can_passthrough_pdf: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub job_id: String,
    pub target_id: String,
    pub destination_path: String,
    pub scope: ExportScope,
    pub mode: ExportContentMode,
    pub include_summary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportProgress {
    pub phase: String,
    pub completed: u32,
    pub total: u32,
}

pub type ExportProgressCallback = Arc<dyn Fn(ExportProgress) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct PreparedExportChapter {
    pub item_id: String,
    pub title: String,
    pub author: Option<String>,
    pub source_title: Option<String>,
    pub url: Option<String>,
    pub published_at: i64,
    pub chapter_level: u32,
    pub document: Document,
    pub translations: HashMap<String, String>,
    pub summary: Option<Summary>,
}

#[derive(Clone, Debug)]
pub struct PreparedPdfExport {
    pub title: String,
    pub author: Option<String>,
    pub mode: ExportContentMode,
    pub chapters: Vec<PreparedExportChapter>,
}

#[derive(Clone, Debug)]
pub enum PdfExportOutcome {
    CopiedOriginal,
    Prepared(PreparedPdfExport),
}

struct PreparedBundle {
    title: String,
    author: Option<String>,
    language: String,
    cover_src: Option<String>,
    chapters: Vec<PreparedExportChapter>,
}

pub fn sanitize_filename(raw: &str) -> String {
    let mut cleaned: String = raw
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == ' ' || c == '-');
    let stem: String = trimmed.chars().take(120).collect();
    if stem.is_empty() {
        "Export".into()
    } else {
        stem
    }
}

fn sniff_image_mime_and_ext(bytes: &[u8], declared_mime: &str) -> (&'static str, &'static str) {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return ("image/png", "png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return ("image/jpeg", "jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return ("image/gif", "gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return ("image/webp", "webp");
    }
    let head = std::str::from_utf8(&bytes[..bytes.len().min(256)]).unwrap_or("");
    if head.contains("<svg") || declared_mime.contains("svg") {
        return ("image/svg+xml", "svg");
    }
    let lower = declared_mime.to_ascii_lowercase();
    if lower.contains("png") {
        ("image/png", "png")
    } else if lower.contains("gif") {
        ("image/gif", "gif")
    } else if lower.contains("webp") {
        ("image/webp", "webp")
    } else {
        ("image/jpeg", "jpg")
    }
}

impl Inner {
    fn check_export_cancelled(&self, job_id: &str) -> Result<()> {
        if self.export_cancels.lock().contains(job_id) {
            return Err(EngineError::Invalid("Export canceled.".into()));
        }
        Ok(())
    }

    fn clear_export_job(&self, job_id: &str) {
        self.export_cancels.lock().remove(job_id);
        self.export_translating.lock().remove(job_id);
    }
}

impl Engine {
    pub fn cancel_export(&self, job_id: &str) {
        self.inner.export_cancels.lock().insert(job_id.to_string());
        if let Some(item_id) = self.inner.export_translating.lock().get(job_id).cloned() {
            self.cancel_translation(&item_id);
        }
    }

    /// Inspects whether `target_id` (a series ID or an item ID) can be exported
    /// to PDF / EPUB, and returns default filenames and option availability.
    pub fn export_options(&self, target_id: &str, from_reader: bool) -> Result<ExportOptions> {
        let inner = &self.inner;
        if let Some(series) = inner.store.series(target_id)? {
            let chapters = inner.chapters(&series.id)?;
            if chapters.is_empty() {
                return Ok(ExportOptions::ineligible(series.title));
            }
            let is_book = series.toc_url.is_none();
            let has_cached_summary_whole = chapters
                .iter()
                .any(|r| self.cached_summary(&r.item.id).ok().flatten().is_some());
            let info = inner.series_info(series.clone())?;
            return Ok(ExportOptions {
                exportable: true,
                series_id: Some(series.id),
                chapter_item_id: None,
                series_title: Some(series.title.clone()),
                item_title: series.title.clone(),
                whole_filename: sanitize_filename(&series.title),
                chapter_filename: None,
                can_choose_scope: false,
                is_book,
                chapter_count: info.chapters,
                has_cached_summary_whole,
                has_cached_summary_chapter: false,
                can_passthrough_epub: is_book,
                can_passthrough_pdf: false,
            });
        }

        let Some(row) = inner.store.item(target_id)? else {
            return Err(EngineError::NotFound);
        };

        // Exclude Collection Media and Gallery / media RSS items.
        if row.item.kind == ItemKind::Saved
            && row.item.series_id.is_none()
            && row
                .item
                .url
                .as_deref()
                .is_some_and(eddy_media::is_media_url)
        {
            return Ok(ExportOptions::ineligible(row.item.title));
        }
        if row.item.kind == ItemKind::Feed
            && (self.in_gallery(&row)?
                || row
                    .item
                    .url
                    .as_deref()
                    .is_some_and(eddy_media::is_media_url))
        {
            return Ok(ExportOptions::ineligible(row.item.title));
        }

        if let Some(series_id) = row.item.series_id.clone()
            && let Some(series) = inner.store.series(&series_id)?
        {
            let chapters = inner.chapters(&series.id)?;
            if chapters.is_empty() {
                return Ok(ExportOptions::ineligible(row.item.title));
            }
            let is_book = series.toc_url.is_none();
            let can_choose_scope = from_reader && chapters.len() > 1;
            let has_cached_summary_whole = chapters
                .iter()
                .any(|r| self.cached_summary(&r.item.id).ok().flatten().is_some());
            let has_cached_summary_chapter = self.cached_summary(&row.item.id)?.is_some();
            let chapter_filename = can_choose_scope
                .then(|| sanitize_filename(&format!("{} - {}", series.title, row.item.title)));
            return Ok(ExportOptions {
                exportable: true,
                series_id: Some(series.id),
                chapter_item_id: Some(row.item.id),
                series_title: Some(series.title.clone()),
                item_title: row.item.title,
                whole_filename: sanitize_filename(&series.title),
                chapter_filename,
                can_choose_scope,
                is_book,
                chapter_count: chapters.len() as u32,
                has_cached_summary_whole,
                has_cached_summary_chapter,
                can_passthrough_epub: is_book,
                can_passthrough_pdf: false,
            });
        }

        let is_pdf = row.format == ContentFormat::Pdf;
        if !is_pdf {
            let has_blocks = inner
                .current_document(&row.item.id)?
                .is_some_and(|d| !d.document.blocks.is_empty());
            if !has_blocks
                && row.item.text_len == 0
                && !inner.full_text_on_open(&row.item.id).unwrap_or(false)
            {
                return Ok(ExportOptions::ineligible(row.item.title));
            }
        }

        let has_summary = self.cached_summary(&row.item.id)?.is_some();
        Ok(ExportOptions {
            exportable: true,
            series_id: None,
            chapter_item_id: Some(row.item.id),
            series_title: None,
            whole_filename: sanitize_filename(&row.item.title),
            item_title: row.item.title,
            chapter_filename: None,
            can_choose_scope: false,
            is_book: false,
            chapter_count: 1,
            has_cached_summary_whole: has_summary,
            has_cached_summary_chapter: has_summary,
            can_passthrough_epub: false,
            can_passthrough_pdf: is_pdf,
        })
    }

    /// Exports `request.target_id` to an EPUB 3 file at `request.destination_path`.
    /// Uses lossless raw `.epub` passthrough when exporting an entire imported Book
    /// in Original mode without extra summary overlay.
    pub async fn export_epub(
        &self,
        request: ExportRequest,
        on_progress: Option<ExportProgressCallback>,
    ) -> Result<()> {
        let job_id = request.job_id.clone();
        self.inner.export_cancels.lock().remove(&job_id);
        let result = self.export_epub_inner(&request, on_progress).await;
        self.inner.clear_export_job(&job_id);
        result
    }

    async fn export_epub_inner(
        &self,
        request: &ExportRequest,
        on_progress: Option<ExportProgressCallback>,
    ) -> Result<()> {
        let options = self.export_options(&request.target_id, true)?;
        if !options.exportable {
            return Err(EngineError::Invalid(
                "This item does not support Reader export.".into(),
            ));
        }
        let report = |phase: &str, completed: u32, total: u32| {
            if let Some(cb) = &on_progress {
                cb(ExportProgress {
                    phase: phase.to_string(),
                    completed,
                    total,
                });
            }
        };

        // Lossless passthrough: Book -> EPUB in Original + Whole + no extra summary.
        if options.can_passthrough_epub
            && request.mode == ExportContentMode::Original
            && request.scope == ExportScope::Whole
            && !request.include_summary
            && let Some(series_id) = &options.series_id
            && let Some(raw_epub) = self.raw_series_epub_path(series_id).await?
        {
            self.inner.check_export_cancelled(&request.job_id)?;
            report("Copying original EPUB…", 1, 1);
            copy_to_destination(&raw_epub, Path::new(&request.destination_path))?;
            return Ok(());
        }

        let bundle = self
            .prepare_export_bundle(request, &options, &on_progress)
            .await?;
        self.inner.check_export_cancelled(&request.job_id)?;

        // Resolve and embed all referenced images (plus cover if any).
        let mut image_urls: Vec<String> = Vec::new();
        if let Some(cover) = &bundle.cover_src {
            image_urls.push(cover.clone());
        }
        for ch in &bundle.chapters {
            for block in &ch.document.blocks {
                if let BlockKind::Image { src, .. } = &block.kind
                    && !image_urls.contains(src)
                {
                    image_urls.push(src.clone());
                }
            }
        }

        let mut images: HashMap<String, EmbeddedImage> = HashMap::new();
        let total_imgs = image_urls.len() as u32;
        for (idx, src) in image_urls.iter().enumerate() {
            self.inner.check_export_cancelled(&request.job_id)?;
            report(
                &format!("Embedding image {} of {total_imgs}…", idx + 1),
                idx as u32,
                total_imgs,
            );
            match self.inner.image(src, None).await {
                Ok((bytes, declared_mime)) => {
                    let (mime, ext) = sniff_image_mime_and_ext(&bytes, &declared_mime);
                    let archive_path = format!("images/img_{:04}.{ext}", idx + 1);
                    images.insert(
                        src.clone(),
                        EmbeddedImage {
                            archive_path,
                            mime: mime.to_string(),
                            bytes,
                        },
                    );
                }
                Err(e) => {
                    self.inner.platform.log(
                        LogLevel::Info,
                        &format!("export skipped unreachable image {src}: {e}"),
                    );
                }
            }
        }

        self.inner.check_export_cancelled(&request.job_id)?;
        report("Packaging EPUB…", 1, 1);

        let item_to_xhtml: HashMap<String, String> = bundle
            .chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| (ch.item_id.clone(), format!("chapter_{:03}.xhtml", i + 1)))
            .collect();

        let engine_ref = self;
        let resolve_link = |url: &str| engine_ref.item_for_link(url).ok().flatten();
        let epub_bytes = build_epub_archive(&EpubPackageInput {
            title: &bundle.title,
            author: bundle.author.as_deref(),
            language: &bundle.language,
            mode: request.mode,
            chapters: &bundle.chapters,
            images: &images,
            cover_src: bundle.cover_src.as_deref(),
            item_to_xhtml: &item_to_xhtml,
            resolve_book_link: &resolve_link,
        })?;

        self.inner.check_export_cancelled(&request.job_id)?;
        write_bytes_to_destination(&epub_bytes, Path::new(&request.destination_path))
    }

    /// Exports `request.target_id` to PDF when raw passthrough applies (`Pdf -> PDF`
    /// in Original mode without summary), or prepares all chapters, translations,
    /// summaries, and cached images so the host platform's native PDF renderer can
    /// typeset it.
    pub async fn export_or_prepare_pdf(
        &self,
        request: ExportRequest,
        on_progress: Option<ExportProgressCallback>,
    ) -> Result<PdfExportOutcome> {
        let job_id = request.job_id.clone();
        self.inner.export_cancels.lock().remove(&job_id);
        let result = self
            .export_or_prepare_pdf_inner(&request, on_progress)
            .await;
        self.inner.clear_export_job(&job_id);
        result
    }

    async fn export_or_prepare_pdf_inner(
        &self,
        request: &ExportRequest,
        on_progress: Option<ExportProgressCallback>,
    ) -> Result<PdfExportOutcome> {
        let options = self.export_options(&request.target_id, true)?;
        if !options.exportable {
            return Err(EngineError::Invalid(
                "This item does not support Reader export.".into(),
            ));
        }
        let report = |phase: &str, completed: u32, total: u32| {
            if let Some(cb) = &on_progress {
                cb(ExportProgress {
                    phase: phase.to_string(),
                    completed,
                    total,
                });
            }
        };

        // Lossless passthrough: Pdf -> PDF in Original + Whole + no extra summary.
        if options.can_passthrough_pdf
            && request.mode == ExportContentMode::Original
            && request.scope == ExportScope::Whole
            && !request.include_summary
            && let Some(item_id) = &options.chapter_item_id
            && let Some(raw_pdf) = self.content_file(item_id).await?
        {
            self.inner.check_export_cancelled(&request.job_id)?;
            report("Copying original PDF…", 1, 1);
            copy_to_destination(&raw_pdf, Path::new(&request.destination_path))?;
            return Ok(PdfExportOutcome::CopiedOriginal);
        }

        let bundle = self
            .prepare_export_bundle(request, &options, &on_progress)
            .await?;
        self.inner.check_export_cancelled(&request.job_id)?;

        // Pre-warm images into the blob cache while the cancellable progress sheet is active.
        let mut image_urls: Vec<String> = Vec::new();
        for ch in &bundle.chapters {
            for block in &ch.document.blocks {
                if let BlockKind::Image { src, .. } = &block.kind
                    && !image_urls.contains(src)
                {
                    image_urls.push(src.clone());
                }
            }
        }
        let total_imgs = image_urls.len() as u32;
        for (idx, src) in image_urls.iter().enumerate() {
            self.inner.check_export_cancelled(&request.job_id)?;
            report(
                &format!("Loading image {} of {total_imgs}…", idx + 1),
                idx as u32,
                total_imgs,
            );
            let _ = self.inner.image(src, None).await;
        }

        self.inner.check_export_cancelled(&request.job_id)?;
        report("Rendering PDF…", 0, 1);

        Ok(PdfExportOutcome::Prepared(PreparedPdfExport {
            title: bundle.title,
            author: bundle.author,
            mode: request.mode,
            chapters: bundle.chapters,
        }))
    }

    async fn raw_series_epub_path(&self, series_id: &str) -> Result<Option<std::path::PathBuf>> {
        let chapters = self.inner.chapters(series_id)?;
        let Some(first) = chapters.first() else {
            return Ok(None);
        };
        let Some(version_hash) = first.item.current_version.as_deref() else {
            return Ok(None);
        };
        let Some(version) = self.inner.store.content_version(version_hash)? else {
            return Ok(None);
        };
        if version.raw_mime != EPUB_MIME {
            return Ok(None);
        }
        self.inner.local_blob_file(&version.raw_blob).await
    }

    async fn prepare_export_bundle(
        &self,
        request: &ExportRequest,
        options: &ExportOptions,
        on_progress: &Option<ExportProgressCallback>,
    ) -> Result<PreparedBundle> {
        let inner = &self.inner;
        let report = |phase: &str, completed: u32, total: u32| {
            if let Some(cb) = on_progress {
                cb(ExportProgress {
                    phase: phase.to_string(),
                    completed,
                    total,
                });
            }
        };

        let (bundle_title, bundle_author, cover_src, rows): (
            String,
            Option<String>,
            Option<String>,
            Vec<ItemRow>,
        ) = if request.scope == ExportScope::Whole
            && let Some(series_id) = &options.series_id
        {
            let series = inner
                .store
                .series(series_id)?
                .ok_or(EngineError::NotFound)?;
            let info = inner.series_info(series)?;
            let chapters = inner.chapters(series_id)?;
            (info.title, info.author, info.cover, chapters)
        } else {
            let item_id = options
                .chapter_item_id
                .as_deref()
                .unwrap_or(&request.target_id);
            let row = inner.store.item(item_id)?.ok_or(EngineError::NotFound)?;
            let author = row.item.author.clone().or_else(|| row.source_title.clone());
            (row.item.title.clone(), author, None, vec![row])
        };

        let total_chapters = rows.len() as u32;
        let mut prepared_chapters = Vec::with_capacity(rows.len());
        let mut detected_lang: Option<String> = None;

        for (idx, row) in rows.into_iter().enumerate() {
            inner.check_export_cancelled(&request.job_id)?;
            let item_id = row.item.id.clone();

            if row.item.kind == ItemKind::Feed && inner.full_text_on_open(&item_id).unwrap_or(false)
            {
                report("Fetching full article…", idx as u32, total_chapters);
                if let Err(e) = inner.fetch_full_text(&item_id, true).await {
                    inner.platform.log(
                        LogLevel::Info,
                        &format!("export full-text fetch skipped for {item_id}: {e}"),
                    );
                }
            } else if row.item.current_version.is_none() {
                report(
                    &format!("Fetching chapter {} of {total_chapters}…", idx + 1),
                    idx as u32,
                    total_chapters,
                );
            }

            let loaded = self.document(&item_id).await?;
            inner.check_export_cancelled(&request.job_id)?;

            let Some(loaded) = loaded else {
                if total_chapters == 1 {
                    return Err(EngineError::Invalid(
                        "This item has no readable text to export.".into(),
                    ));
                }
                continue;
            };

            if loaded.document.blocks.is_empty() {
                if total_chapters == 1 {
                    let msg = if loaded.version.raw_mime == PDF_MIME {
                        "This PDF has no extractable text for reflowed or translated export."
                    } else {
                        "This item has no readable text to export."
                    };
                    return Err(EngineError::Invalid(msg.into()));
                }
                continue;
            }

            // Refresh row metadata in case fetching populated `lang` or `title`.
            let fresh_row = inner.store.item(&item_id)?.unwrap_or(row);
            if detected_lang.is_none() {
                detected_lang = fresh_row.item.lang.clone();
            }

            let translations = match request.mode {
                ExportContentMode::Original => HashMap::new(),
                ExportContentMode::Bilingual | ExportContentMode::Translation => {
                    if let Some(cached) = self.cached_translation(&item_id)?
                        && cached.complete
                    {
                        cached.segments
                    } else {
                        inner
                            .export_translating
                            .lock()
                            .insert(request.job_id.clone(), item_id.clone());
                        let label = if total_chapters > 1 {
                            format!("Translating chapter {} of {total_chapters}…", idx + 1)
                        } else {
                            "Translating…".to_string()
                        };
                        report(&label, idx as u32, total_chapters);
                        let progress_cb: Option<BatchCallback> = on_progress.as_ref().map(|cb| {
                            let cb = cb.clone();
                            let label = label.clone();
                            let idx = idx as u32;
                            Arc::new(move |_batch: &HashMap<String, String>| {
                                cb(ExportProgress {
                                    phase: label.clone(),
                                    completed: idx,
                                    total: total_chapters,
                                });
                            }) as BatchCallback
                        });
                        let res = self.translate(&item_id, false, progress_cb).await;
                        inner.export_translating.lock().remove(&request.job_id);
                        inner.check_export_cancelled(&request.job_id)?;
                        let translated = res?;
                        if !translated.complete {
                            return Err(EngineError::Invalid("Export canceled.".into()));
                        }
                        translated.segments
                    }
                }
            };

            let summary = if request.include_summary {
                self.cached_summary(&item_id)?
            } else {
                None
            };

            let chapter_level = if total_chapters > 1 {
                fresh_row.item.chapter_level
            } else {
                0
            };

            prepared_chapters.push(PreparedExportChapter {
                item_id,
                title: fresh_row.item.title,
                author: fresh_row.item.author,
                source_title: fresh_row.source_title,
                url: fresh_row.item.url,
                published_at: fresh_row.item.published_at,
                chapter_level,
                document: loaded.document,
                translations,
                summary,
            });
        }

        if prepared_chapters.is_empty() {
            return Err(EngineError::Invalid(
                "No readable chapters were found to export.".into(),
            ));
        }

        let language = if request.mode == ExportContentMode::Translation {
            self.ai_settings()
                .map(|s| s.target_language)
                .unwrap_or_else(|_| "en".into())
        } else {
            detected_lang.unwrap_or_else(|| "en".into())
        };

        Ok(PreparedBundle {
            title: bundle_title,
            author: bundle_author,
            language,
            cover_src,
            chapters: prepared_chapters,
        })
    }
}

fn ensure_parent_dir(dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| EngineError::Invalid(format!("Could not create export folder: {e}")))?;
    }
    Ok(())
}

fn copy_to_destination(src: &Path, dest: &Path) -> Result<()> {
    ensure_parent_dir(dest)?;
    std::fs::copy(src, dest)
        .map_err(|e| EngineError::Invalid(format!("Could not write exported file: {e}")))?;
    Ok(())
}

fn write_bytes_to_destination(bytes: &[u8], dest: &Path) -> Result<()> {
    ensure_parent_dir(dest)?;
    std::fs::write(dest, bytes)
        .map_err(|e| EngineError::Invalid(format!("Could not write exported file: {e}")))
}
