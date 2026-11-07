use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

use super::*;

const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

pub(crate) fn archive(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(MIME.as_bytes()).unwrap();
    for (name, body) in files {
        zip.start_file(*name, deflated).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn open(bytes: Vec<u8>) -> Epub<Cursor<Vec<u8>>> {
    Epub::new(Cursor::new(bytes)).unwrap()
}

fn epub3() -> Vec<u8> {
    archive(&[
        ("META-INF/container.xml", CONTAINER),
        (
            "OEBPS/content.opf",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>  Ray   Tracing </dc:title>
    <dc:creator>Peter Shirley</dc:creator><dc:creator>Trevor Black</dc:creator>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cover" href="text/cover.xhtml" media-type="application/xhtml+xml"/>
    <item id="c1" href="text/ch%201.xhtml" media-type="application/xhtml+xml"/>
    <item id="c1b" href="text/ch1_split.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="notes" href="text/notes.xhtml" media-type="application/xhtml+xml"/>
    <item id="img" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
  </manifest>
  <spine>
    <itemref idref="cover"/><itemref idref="c1"/><itemref idref="c1b"/><itemref idref="c2"/>
    <itemref idref="notes" linear="no"/>
  </spine>
</package>"#,
        ),
        (
            "OEBPS/nav.xhtml",
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="landmarks"><ol><li><a href="text/cover.xhtml">Cover</a></li></ol></nav>
<nav epub:type="toc"><ol>
  <li><a href="text/ch%201.xhtml">Chapter&#160;1:
     Overview</a><ol><li><a href="text/ch%201.xhtml#s1">Sub</a></li></ol></li>
  <li><a href="text/ch2.xhtml">Chapter 2</a></li>
</ol></nav></body></html>"#,
        ),
        (
            "OEBPS/text/cover.xhtml",
            r#"<html><body><svg><image width="600" xlink:href="../images/cover.jpg"/></svg></body></html>"#,
        ),
        (
            "OEBPS/text/ch 1.xhtml",
            r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title/>
<link href="../styles/a.css" rel="stylesheet"/></head>
<body class="c"><h1>Chapter 1</h1><p>See <a href="ch2.xhtml#n1">the note</a>.<a id="p1"/> After the anchor.</p>
<img src="../images/fig%20one.png" alt="Fig"/><p><a href="https://example.com/">Web</a></p></body></html>"#,
        ),
        (
            "OEBPS/text/ch1_split.xhtml",
            "<html><body><p>Second half.</p></body></html>",
        ),
        (
            "OEBPS/text/ch2.xhtml",
            "<html><body><h1>Chapter 2</h1><p id='n1'>Note.</p></body></html>",
        ),
        (
            "OEBPS/text/notes.xhtml",
            "<html><body><p>Notes</p></body></html>",
        ),
        ("OEBPS/images/cover.jpg", "jpeg"),
    ])
}

#[test]
fn epub3_metadata_and_chapters_follow_the_table_of_contents() {
    let mut book = open(epub3());
    let b = book.book().unwrap();
    assert_eq!(b.title.as_deref(), Some("Ray Tracing"));
    assert_eq!(b.author.as_deref(), Some("Peter Shirley, Trevor Black"));
    assert_eq!(b.language.as_deref(), Some("en"));
    assert_eq!(b.cover.as_deref(), Some("OEBPS/images/cover.jpg"));
    assert_eq!(
        b.chapters,
        vec![
            Chapter {
                title: None,
                parts: vec!["OEBPS/text/cover.xhtml".into()]
            },
            Chapter {
                title: Some("Chapter 1: Overview".into()),
                parts: vec![
                    "OEBPS/text/ch 1.xhtml".into(),
                    "OEBPS/text/ch1_split.xhtml".into()
                ]
            },
            Chapter {
                title: Some("Chapter 2".into()),
                parts: vec!["OEBPS/text/ch2.xhtml".into()]
            },
        ],
        "split files join their chapter; non-linear notes are left out"
    );
    assert_eq!(book.read("OEBPS/images/cover.jpg").unwrap(), b"jpeg");
    assert_eq!(
        book.read("oebps/IMAGES/cover.jpg").unwrap(),
        b"jpeg",
        "case-insensitive fallback"
    );
    assert!(book.read("OEBPS/missing.png").is_none());
}

#[test]
fn chapter_html_is_safe_for_html_parsers_and_uses_absolute_references() {
    let mut book = open(epub3());
    let b = book.book().unwrap();
    let html = book.chapter_html(&b.chapters[1]);
    assert!(html.contains("<h1>Chapter 1</h1>"));
    assert!(html.contains("Second half."), "parts are joined");
    assert!(!html.contains("<title"), "only bodies are kept");
    assert!(html.contains(r#"<a id="p1"></a> After the anchor."#));
    assert!(html.contains(r#"href="/OEBPS/text/ch2.xhtml#n1""#));
    assert!(html.contains(r#"src="/OEBPS/images/fig%20one.png""#));
    assert!(html.contains(r#"href="https://example.com/""#));

    let cover = book.chapter_html(&b.chapters[0]);
    assert!(
        cover.contains(r#"<img src="/OEBPS/images/cover.jpg">"#),
        "{cover}"
    );
}

#[test]
fn epub2_uses_the_ncx_and_cover_meta() {
    let mut book = open(archive(&[
        ("META-INF/container.xml", CONTAINER),
        (
            "OEBPS/content.opf",
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>光线追踪</dc:title><meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="a" href="a.html" media-type="application/xhtml+xml"/>
    <item id="b" href="b.html" media-type="application/xhtml+xml"/>
    <item id="cover-img" href="img/front.png" media-type="image/png"/>
  </manifest>
  <spine toc="ncx"><itemref idref="a"/><itemref idref="b"/></spine>
</package>"#,
        ),
        (
            "OEBPS/toc.ncx",
            r#"<?xml version="1.0"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>
  <navPoint id="p1"><navLabel><text>第一章</text></navLabel><content src="a.html"/>
    <navPoint id="p1a"><navLabel><text>小节</text></navLabel><content src="a.html#x"/></navPoint>
  </navPoint>
  <navPoint id="p2"><navLabel><text>第二章</text></navLabel><content src="b.html"/></navPoint>
</navMap></ncx>"#,
        ),
        ("OEBPS/a.html", "<html><body><p>一</p></body></html>"),
        ("OEBPS/b.html", "<html><body><p>二</p></body></html>"),
    ]));
    let b = book.book().unwrap();
    assert_eq!(b.title.as_deref(), Some("光线追踪"));
    assert_eq!(b.cover.as_deref(), Some("OEBPS/img/front.png"));
    let titles: Vec<_> = b.chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(titles, [Some("第一章"), Some("第二章")]);
}

#[test]
fn without_a_table_of_contents_every_file_is_a_chapter() {
    let mut book = open(archive(&[
        ("META-INF/container.xml", CONTAINER),
        (
            "OEBPS/content.opf",
            r#"<package><metadata/><manifest>
  <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
  <item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>
</manifest><spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#,
        ),
        ("OEBPS/a.xhtml", "<p>a</p>"),
        ("OEBPS/b.xhtml", "<p>b</p>"),
    ]));
    let b = book.book().unwrap();
    assert_eq!(b.title, None);
    assert_eq!(b.chapters.len(), 2);
    assert!(b.chapters.iter().all(|c| c.title.is_none()));
}

#[test]
fn drm_is_rejected_but_font_obfuscation_is_not() {
    let enc = |algorithm: &str| {
        format!(
            r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <enc:EncryptedData><enc:EncryptionMethod Algorithm="{algorithm}"/>
  <enc:CipherData><enc:CipherReference URI="OEBPS/a.xhtml"/></enc:CipherData></enc:EncryptedData>
</encryption>"#
        )
    };
    let opf = r#"<package><manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#;
    let drm = enc("http://www.w3.org/2001/04/xmlenc#aes128-cbc");
    let mut book = open(archive(&[
        ("META-INF/container.xml", CONTAINER),
        ("META-INF/encryption.xml", &drm),
        ("OEBPS/content.opf", opf),
        ("OEBPS/a.xhtml", "<p>a</p>"),
    ]));
    assert!(matches!(book.book(), Err(EpubError::Encrypted)));

    let fonts = enc("http://www.idpf.org/2008/embedding");
    let mut book = open(archive(&[
        ("META-INF/container.xml", CONTAINER),
        ("META-INF/encryption.xml", &fonts),
        ("OEBPS/content.opf", opf),
        ("OEBPS/a.xhtml", "<p>a</p>"),
    ]));
    assert!(book.book().is_ok());
}

#[test]
fn not_an_epub() {
    assert!(Epub::new(Cursor::new(b"%PDF-1.7".to_vec())).is_err());
    let mut empty = open(archive(&[]));
    assert!(matches!(empty.book(), Err(EpubError::Invalid(_))));
}

#[test]
fn archive_paths_resolve_like_urls() {
    assert_eq!(
        join("OEBPS/text/a.xhtml", "../images/x%20y.png#f").as_deref(),
        Some("OEBPS/images/x y.png")
    );
    assert_eq!(
        join("OEBPS/a.xhtml", "/abs/b.png").as_deref(),
        Some("abs/b.png")
    );
    assert_eq!(
        join("OEBPS/a.xhtml", "./b.xhtml?x=1").as_deref(),
        Some("OEBPS/b.xhtml")
    );
    assert_eq!(join("a.xhtml", "../../x.png"), None, "outside the archive");
    assert_eq!(join("OEBPS/a.xhtml", "https://example.com/x.png"), None);
    assert_eq!(join("OEBPS/a.xhtml", "mailto:me@example.com"), None);
    assert_eq!(join("OEBPS/a.xhtml", "#frag"), None);
    let path = "OEBPS/图 1/a b#.png";
    assert_eq!(encode_path(path), "OEBPS/%E5%9B%BE%201/a%20b%23.png");
    assert_eq!(decode_path(&format!("/{}", encode_path(path))), path);
}
