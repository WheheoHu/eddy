use std::path::{Path, PathBuf};

use crate::{EngineError, Result};

/// Name the Chrome extension uses in `chrome.runtime.sendNativeMessage`.
pub const NATIVE_HOST_NAME: &str = "dev.eddy.capture";

/// Derived from the public `key` in `extensions/chrome/manifest.json`; the
/// key pins the ID so the host can allow exactly this extension.
pub const CHROME_EXTENSION_ID: &str = "cmmebhdhipeakdgcgdddiadkkacoecpb";

/// Browser profile roots (under `~/Library/Application Support`) that read
/// Chrome-style native messaging host manifests.
const BROWSERS: &[&str] = &[
    "Google/Chrome",
    "Google/Chrome Beta",
    "Chromium",
    "Microsoft Edge",
    "BraveSoftware/Brave-Browser",
    "Arc/User Data",
];

/// Writes the native messaging manifest pointing at `host_path` for Chrome
/// and every other installed Chromium browser. Returns the files written.
pub fn install_native_host(host_path: &Path) -> Result<Vec<PathBuf>> {
    if !host_path.is_file() {
        return Err(EngineError::Invalid(format!("找不到捕获程序：{}", host_path.display())));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| EngineError::Invalid("找不到用户目录。".into()))?;
    let support = home.join("Library/Application Support");
    let manifest = serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Eddy page capture",
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{CHROME_EXTENSION_ID}/")],
    });
    let body = serde_json::to_vec_pretty(&manifest)?;
    let mut written = Vec::new();
    for (i, browser) in BROWSERS.iter().enumerate() {
        let root = support.join(browser);
        // Always install for Chrome; others only when the browser exists.
        if i != 0 && !root.exists() {
            continue;
        }
        let dir = root.join("NativeMessagingHosts");
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Invalid(e.to_string()))?;
        let file = dir.join(format!("{NATIVE_HOST_NAME}.json"));
        std::fs::write(&file, &body).map_err(|e| EngineError::Invalid(e.to_string()))?;
        written.push(file);
    }
    Ok(written)
}
