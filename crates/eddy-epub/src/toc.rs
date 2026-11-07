use scraper::{ElementRef, Html, Selector};

use crate::paths::join;
use crate::{collapse, parse_xml};

/// Entries of an EPUB 3 navigation document's table of contents, in
/// document order: (archive path, title).
pub(crate) fn nav_entries(text: &str, nav_path: &str) -> Vec<(String, String)> {
    let html = Html::parse_document(text);
    let navs = Selector::parse("nav").expect("valid selector");
    let links = Selector::parse("a[href]").expect("valid selector");
    let navs: Vec<ElementRef> = html.select(&navs).collect();
    let toc = navs
        .iter()
        .find(|nav| {
            nav.value().attrs().any(|(name, value)| {
                name.ends_with("type") && value.split_whitespace().any(|t| t == "toc")
            })
        })
        .or(navs.first());
    let Some(toc) = toc else {
        return Vec::new();
    };
    toc.select(&links)
        .filter_map(|a| {
            let path = join(nav_path, a.value().attr("href")?)?;
            let title = collapse(&a.text().collect::<String>())?;
            Some((path, title))
        })
        .collect()
}

/// Entries of an EPUB 2 NCX table of contents, in document order.
pub(crate) fn ncx_entries(text: &str, ncx_path: &str) -> Vec<(String, String)> {
    let Ok(doc) = parse_xml(text) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "navPoint")
        .filter_map(|point| {
            let label = point
                .children()
                .find(|c| c.tag_name().name() == "navLabel")?
                .descendants()
                .find(|c| c.tag_name().name() == "text")?
                .text()?;
            let src = point
                .children()
                .find(|c| c.tag_name().name() == "content")?
                .attribute("src")?;
            Some((join(ncx_path, src)?, collapse(label)?))
        })
        .collect()
}
