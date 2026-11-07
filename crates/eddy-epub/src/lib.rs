//! EPUB container parsing for imports: book metadata, the reading order
//! grouped into chapters by the table of contents, and the files inside the
//! archive. Turning chapter HTML into documents is `eddy-doc`'s job.

mod paths;
mod toc;

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use zip::ZipArchive;

pub use paths::{decode_path, encode_path, join};

pub const MIME: &str = "application/epub+zip";

/// Largest archive entry read into memory.
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// `EncryptionMethod` algorithms that only obfuscate embedded fonts.
const FONT_OBFUSCATION: [&str; 2] = [
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

#[derive(Debug, thiserror::Error)]
pub enum EpubError {
    #[error("不是有效的 EPUB 文件：{0}")]
    Invalid(String),
    #[error("这本 EPUB 有 DRM 加密，无法导入。")]
    Encrypted,
}

pub type Result<T> = std::result::Result<T, EpubError>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    /// Archive path of the cover image.
    pub cover: Option<String>,
    pub chapters: Vec<Chapter>,
}

/// A table-of-contents entry with the reading-order files up to the next
/// entry: books often split one chapter over several files.
#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub title: Option<String>,
    /// Archive paths, in reading order.
    pub parts: Vec<String>,
}

pub struct Epub<R> {
    zip: ZipArchive<R>,
}

impl<R: Read + Seek> Epub<R> {
    pub fn new(reader: R) -> Result<Self> {
        let zip = ZipArchive::new(reader).map_err(|e| EpubError::Invalid(e.to_string()))?;
        Ok(Epub { zip })
    }

    /// A file in the archive. Paths match case-insensitively as a fallback:
    /// books are often made on case-insensitive file systems.
    pub fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let name = if self.zip.index_for_name(path).is_some() {
            path.to_string()
        } else {
            self.zip
                .file_names()
                .find(|n| n.eq_ignore_ascii_case(path))?
                .to_string()
        };
        let file = self.zip.by_name(&name).ok()?;
        if file.size() > MAX_ENTRY_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.take(MAX_ENTRY_BYTES).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    }

    fn read_text(&mut self, path: &str) -> Option<String> {
        self.read(path).map(|b| decode(&b))
    }

    pub fn book(&mut self) -> Result<Book> {
        self.check_encryption()?;
        let container = self
            .read_text("META-INF/container.xml")
            .ok_or_else(|| invalid("缺少 META-INF/container.xml"))?;
        let opf_path = rootfile(&container).ok_or_else(|| invalid("找不到内容清单"))?;
        let opf = self
            .read_text(&opf_path)
            .ok_or_else(|| invalid("找不到内容清单"))?;
        let package = Package::parse(&opf, &opf_path)?;

        let mut toc = Vec::new();
        if let Some(nav) = package.manifest.values().find(|i| i.has_property("nav"))
            && let Some(text) = self.read_text(&nav.path)
        {
            toc = toc::nav_entries(&text, &nav.path);
        }
        if toc.is_empty()
            && let Some(ncx) = package
                .ncx_id
                .as_ref()
                .and_then(|id| package.manifest.get(id))
                .or_else(|| {
                    package
                        .manifest
                        .values()
                        .find(|i| i.media_type == "application/x-dtbncx+xml")
                })
            && let Some(text) = self.read_text(&ncx.path)
        {
            toc = toc::ncx_entries(&text, &ncx.path);
        }
        let mut titles: HashMap<String, String> = HashMap::new();
        for (path, title) in toc {
            titles.entry(path).or_insert(title);
        }

        let spine: Vec<String> = package
            .spine
            .iter()
            .filter_map(|id| package.manifest.get(id))
            .filter(|i| i.is_html())
            .map(|i| i.path.clone())
            .collect();
        // Without a usable table of contents every file is its own chapter.
        let grouped = spine.iter().any(|p| titles.contains_key(p));
        let mut chapters: Vec<Chapter> = Vec::new();
        for path in spine {
            let title = titles.get(&path).cloned();
            match chapters.last_mut() {
                Some(last) if grouped && title.is_none() => last.parts.push(path),
                _ => chapters.push(Chapter {
                    title,
                    parts: vec![path],
                }),
            }
        }
        if chapters.is_empty() {
            return Err(invalid("没有可阅读的章节"));
        }
        Ok(Book {
            cover: package.cover(),
            title: package.title,
            author: package.author,
            language: package.language,
            chapters,
        })
    }

    /// The chapter as one HTML document. References are rewritten to
    /// archive-absolute paths (`/OEBPS/images/a.png`), so they resolve the
    /// same from every part against a base URL for any of them.
    pub fn chapter_html(&mut self, chapter: &Chapter) -> String {
        let mut body = String::new();
        for part in &chapter.parts {
            if let Some(text) = self.read_text(part) {
                body.push_str(&part_body(&text, part));
                body.push('\n');
            }
        }
        format!("<!doctype html><html><body>{body}</body></html>")
    }

    /// Font obfuscation is fine; any other encryption is DRM and leaves the
    /// content unreadable.
    fn check_encryption(&mut self) -> Result<()> {
        let Some(text) = self.read_text("META-INF/encryption.xml") else {
            return Ok(());
        };
        let doc = parse_xml(&text).map_err(|_| EpubError::Encrypted)?;
        let drm = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "EncryptionMethod")
            .any(|n| {
                n.attribute("Algorithm")
                    .is_none_or(|a| !FONT_OBFUSCATION.contains(&a))
            });
        if drm {
            Err(EpubError::Encrypted)
        } else {
            Ok(())
        }
    }
}

/// MIME type of an archive file, from its extension.
pub fn mime_for(path: &str) -> &'static str {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "xhtml" | "html" | "htm" => "application/xhtml+xml",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

struct ManifestItem {
    path: String,
    media_type: String,
    properties: String,
}

impl ManifestItem {
    fn has_property(&self, p: &str) -> bool {
        self.properties.split_whitespace().any(|x| x == p)
    }

    fn is_html(&self) -> bool {
        matches!(
            self.media_type.as_str(),
            "application/xhtml+xml" | "text/html" | "application/x-dtbook+xml"
        ) || matches!(mime_for(&self.path), "application/xhtml+xml")
    }

    fn is_image(&self) -> bool {
        self.media_type.starts_with("image/")
    }
}

struct Package {
    title: Option<String>,
    author: Option<String>,
    language: Option<String>,
    cover_id: Option<String>,
    manifest: HashMap<String, ManifestItem>,
    /// Manifest IDs of the linear reading order.
    spine: Vec<String>,
    ncx_id: Option<String>,
}

impl Package {
    fn parse(text: &str, opf_path: &str) -> Result<Package> {
        let doc = parse_xml(text).map_err(|e| invalid(&format!("内容清单格式错误（{e}）")))?;
        let element = |name: &str| doc.descendants().find(|n| n.tag_name().name() == name);
        let metadata = element("metadata");
        let meta_text = |name: &str| -> Vec<String> {
            metadata
                .into_iter()
                .flat_map(|m| m.descendants())
                .filter(|n| n.tag_name().name() == name)
                .filter_map(|n| n.text().and_then(collapse))
                .collect()
        };
        let mut creators = meta_text("creator");
        creators.dedup();
        let cover_id = metadata
            .into_iter()
            .flat_map(|m| m.descendants())
            .find(|n| n.tag_name().name() == "meta" && n.attribute("name") == Some("cover"))
            .and_then(|n| n.attribute("content"))
            .map(str::to_string);

        let manifest = element("manifest")
            .into_iter()
            .flat_map(|m| m.children())
            .filter(|n| n.tag_name().name() == "item")
            .filter_map(|n| {
                let item = ManifestItem {
                    path: join(opf_path, n.attribute("href")?)?,
                    media_type: n.attribute("media-type").unwrap_or_default().to_string(),
                    properties: n.attribute("properties").unwrap_or_default().to_string(),
                };
                Some((n.attribute("id")?.to_string(), item))
            })
            .collect();
        let spine_node = element("spine");
        let spine = spine_node
            .into_iter()
            .flat_map(|s| s.children())
            .filter(|n| n.tag_name().name() == "itemref" && n.attribute("linear") != Some("no"))
            .filter_map(|n| n.attribute("idref").map(str::to_string))
            .collect();
        Ok(Package {
            title: meta_text("title").into_iter().next(),
            author: (!creators.is_empty()).then(|| creators.join(", ")),
            language: meta_text("language").into_iter().next(),
            cover_id,
            manifest,
            spine,
            ncx_id: spine_node
                .and_then(|s| s.attribute("toc"))
                .map(str::to_string),
        })
    }

    fn cover(&self) -> Option<String> {
        let images = || self.manifest.iter().filter(|(_, i)| i.is_image());
        images()
            .find(|(_, i)| i.has_property("cover-image"))
            .or_else(|| {
                let id = self.cover_id.as_deref()?;
                images().find(|(k, _)| k.as_str() == id)
            })
            .or_else(|| {
                images()
                    .filter(|(k, i)| {
                        k.to_ascii_lowercase().contains("cover")
                            || i.path.to_ascii_lowercase().contains("cover")
                    })
                    .min_by_key(|(_, i)| i.path.len())
            })
            .map(|(_, i)| i.path.clone())
    }
}

fn rootfile(container: &str) -> Option<String> {
    let doc = parse_xml(container).ok()?;
    let full_path = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "rootfile")
        .find(|n| {
            n.attribute("media-type")
                .is_none_or(|m| m == "application/oebps-package+xml")
        })?
        .attribute("full-path")?;
    join("", full_path)
}

pub(crate) fn parse_xml(
    text: &str,
) -> std::result::Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
}

/// Whitespace collapsed; `None` when nothing is left.
pub(crate) fn collapse(s: &str) -> Option<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

fn decode(bytes: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if be {
                    u16::from_be_bytes(c)
                } else {
                    u16::from_le_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFE, 0xFF, ..] => utf16(true),
        [0xFF, 0xFE, ..] => utf16(false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn invalid(msg: &str) -> EpubError {
    EpubError::Invalid(msg.to_string())
}

const VOID_ELEMENTS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

static SELF_CLOSING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<([A-Za-z][\w:.-]*)(\s[^<>]*?)?\s*/>").expect("valid regex"));
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(src|href)(\s*=\s*)(?:"([^"]*)"|'([^']*)')"#).expect("valid regex")
});
static SVG_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<image\b[^>]*?\bhref\s*=\s*"([^"]*)"[^>]*>"#).expect("valid regex")
});
static BODY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<body\b[^>]*>(.*)</body\s*>").expect("valid regex"));

/// The body of one XHTML part, made safe for an HTML parser: XML
/// self-closing tags expanded (`<title/>` would otherwise swallow the rest
/// of the file), references made archive-absolute, and SVG cover images
/// turned into plain images.
fn part_body(xhtml: &str, part: &str) -> String {
    let html = SELF_CLOSING.replace_all(xhtml, |c: &Captures| {
        let tag = &c[1];
        let attrs = c.get(2).map_or("", |m| m.as_str());
        if VOID_ELEMENTS.contains(&tag.to_ascii_lowercase().as_str()) {
            format!("<{tag}{attrs}>")
        } else {
            format!("<{tag}{attrs}></{tag}>")
        }
    });
    let html = REFERENCE.replace_all(&html, |c: &Captures| {
        let value = c.get(3).or(c.get(4)).map_or("", |m| m.as_str());
        let fragment = value.find('#').map_or("", |i| &value[i..]);
        match join(part, value) {
            Some(path) if !value.starts_with('#') => {
                format!("{}{}\"/{}{fragment}\"", &c[1], &c[2], encode_path(&path))
            }
            _ => c[0].to_string(),
        }
    });
    let html = SVG_IMAGE.replace_all(&html, "<img src=\"$1\">");
    match BODY.captures(&html) {
        Some(c) => c[1].to_string(),
        None => html.into_owned(),
    }
}

#[cfg(test)]
mod tests;
