//! Chrome native messaging host for the Eddy extension. Each message carries
//! a page the user is looking at (or a bare link); it is written atomically
//! into Eddy's spool directory, which the app imports on its own schedule, so
//! saving works even while Eddy is closed. A `{"show": url}` message opens
//! Eddy at the page saved from `url` instead.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Chrome allows up to 64 MiB from extension to host.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Capture {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    #[serde(default)]
    captured_at: i64,
}

fn spool_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EDDY_SPOOL_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/Eddy/spool")
}

fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn write_message(output: &mut impl Write, value: &serde_json::Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    output.write_all(&(body.len() as u32).to_ne_bytes())?;
    output.write_all(&body)?;
    output.flush()
}

fn store(dir: &Path, raw: &[u8]) -> Result<String, String> {
    let mut capture: Capture =
        serde_json::from_slice(raw).map_err(|_| "invalid message".to_string())?;
    if !(capture.url.starts_with("https://") || capture.url.starts_with("http://")) {
        return Err("only http and https pages can be saved".into());
    }
    if capture.captured_at == 0 {
        capture.captured_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::now_v7().to_string();
    let tmp = dir.join(format!(".{id}.tmp"));
    let target = dir.join(format!("{id}.json"));
    std::fs::write(
        &tmp,
        serde_json::to_vec(&capture).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(id)
}

#[derive(Deserialize)]
struct Show {
    show: String,
}

/// The link that opens Eddy at the page saved from `page`. Eddy imports
/// what is waiting in the spool before it looks.
fn show_link(page: &str) -> Result<String, String> {
    if !(page.starts_with("https://") || page.starts_with("http://")) {
        return Err("only http and https pages can be shown".into());
    }
    let mut link = url::Url::parse("eddy://show").map_err(|e| e.to_string())?;
    link.query_pairs_mut().append_pair("url", page);
    Ok(link.into())
}

fn open_link(link: &str) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(link)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Eddy couldn't be opened".into())
    }
}

fn serve(
    input: &mut impl Read,
    output: &mut impl Write,
    dir: &Path,
    open: impl Fn(&str) -> Result<(), String>,
) -> io::Result<()> {
    while let Some(raw) = read_message(input)? {
        let reply = if let Ok(Show { show }) = serde_json::from_slice(&raw) {
            match show_link(&show).and_then(|link| open(&link)) {
                Ok(()) => serde_json::json!({"ok": true}),
                Err(e) => serde_json::json!({"ok": false, "error": e}),
            }
        } else {
            match store(dir, &raw) {
                Ok(id) => serde_json::json!({"ok": true, "id": id}),
                Err(e) => serde_json::json!({"ok": false, "error": e}),
            }
        };
        write_message(output, &reply)?;
    }
    Ok(())
}

fn main() {
    let dir = spool_dir();
    if let Err(e) = serve(
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &dir,
        open_link,
    ) {
        eprintln!("eddy-capture-host: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: &serde_json::Value) -> Vec<u8> {
        let mut out = Vec::new();
        write_message(&mut out, value).unwrap();
        out
    }

    #[test]
    fn stores_captures_and_replies_per_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut input = frame(
            &serde_json::json!({"url": "https://example.com/a", "title": "A", "html": "<p>x</p>"}),
        );
        input.extend(frame(&serde_json::json!({"url": "javascript:alert(1)"})));
        let mut output = Vec::new();
        serve(&mut input.as_slice(), &mut output, dir.path(), |_| Ok(())).unwrap();

        let mut replies = output.as_slice();
        let first: serde_json::Value =
            serde_json::from_slice(&read_message(&mut replies).unwrap().unwrap()).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&read_message(&mut replies).unwrap().unwrap()).unwrap();
        assert_eq!(first["ok"], true);
        assert_eq!(second["ok"], false);

        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let saved: Capture =
            serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
        assert_eq!(saved.title.as_deref(), Some("A"));
        assert!(saved.captured_at > 0);
    }

    #[test]
    fn show_opens_eddy_at_the_page_and_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut input = frame(&serde_json::json!({"show": "https://example.com/a?b=1&c=2#d"}));
        input.extend(frame(&serde_json::json!({"show": "file:///etc/passwd"})));
        let opened = std::cell::RefCell::new(Vec::new());
        let mut output = Vec::new();
        serve(&mut input.as_slice(), &mut output, dir.path(), |link| {
            opened.borrow_mut().push(link.to_string());
            Ok(())
        })
        .unwrap();

        let mut replies = output.as_slice();
        let first: serde_json::Value =
            serde_json::from_slice(&read_message(&mut replies).unwrap().unwrap()).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&read_message(&mut replies).unwrap().unwrap()).unwrap();
        assert_eq!(first["ok"], true);
        assert_eq!(second["ok"], false);
        let opened = opened.into_inner();
        assert_eq!(opened.len(), 1);
        let link = url::Url::parse(&opened[0]).unwrap();
        assert_eq!((link.scheme(), link.host_str()), ("eddy", Some("show")));
        assert_eq!(
            link.query_pairs().find(|(k, _)| k == "url").unwrap().1,
            "https://example.com/a?b=1&c=2#d"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_oversized_frames() {
        let mut input = ((MAX_MESSAGE + 1) as u32).to_ne_bytes().to_vec();
        input.extend([0u8; 8]);
        assert!(read_message(&mut input.as_slice()).is_err());
    }
}
