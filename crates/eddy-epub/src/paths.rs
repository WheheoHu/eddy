use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// Characters escaped in a URL path segment built from an archive path.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Resolves `href`, as written in the archive file at `base`, to an archive
/// path: percent-decoded, without fragment or query, with `.` and `..`
/// applied. `None` for references to other schemes or outside the archive.
pub fn join(base: &str, href: &str) -> Option<String> {
    let href = href.trim().split(['#', '?']).next()?;
    if href.is_empty() || has_scheme(href) {
        return None;
    }
    let href = percent_decode_str(href).decode_utf8_lossy();
    let mut out: Vec<String> = Vec::new();
    if !href.starts_with('/') {
        out.extend(
            base.split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
        out.pop();
    }
    for segment in href.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s.to_string()),
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

/// An archive path as a URL path (segments escaped, `/` kept).
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// The archive path of an escaped URL path.
pub fn decode_path(path: &str) -> String {
    percent_decode_str(path.trim_start_matches('/'))
        .decode_utf8_lossy()
        .into_owned()
}

fn has_scheme(href: &str) -> bool {
    href.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}
