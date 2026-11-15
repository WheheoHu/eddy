//! Engine construction and the callback bridge to the webview: engine
//! change/progress pulses become Tauri events the frontend debounces.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use eddy_core::{Engine, EngineOptions, Platform};

use crate::platform::WindowsPlatform;

pub struct EngineState {
    pub engine: Engine,
    pub data_dir: PathBuf,
    /// pdfium.dll sits next to the exe; PDF import is offered only when true.
    pub pdfium: bool,
}

/// Data directory for the app: `%LOCALAPPDATA%\Eddy`, overridable with
/// `EDDY_DATA_DIR` so tests and development can point somewhere else.
pub fn resolve_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("EDDY_DATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(local).join("Eddy")
}

/// Opens the engine and reports whether pdfium.dll is present next to the
/// exe (the first place LoadLibrary searches), so PDF import can be offered
/// or hidden. `Err` carries the engine's error text.
pub fn open_engine(data_dir: &std::path::Path) -> Result<(Engine, bool), String> {
    let platform: Arc<dyn Platform> = Arc::new(WindowsPlatform::new(data_dir));
    let engine = Engine::open(data_dir, platform, EngineOptions::default())
        .map_err(|e| e.to_string())?;
    let pdfium_available = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("pdfium.dll").exists()))
        .unwrap_or(false);
    Ok((engine, pdfium_available))
}

pub fn install_listeners(handle: AppHandle, engine: &Engine) {
    let change_handle = handle.clone();
    engine.set_change_listener(Some(Arc::new(move || {
        let _ = change_handle.emit("eddy:change", ());
    })));
    let progress_handle = handle.clone();
    engine.set_progress_listener(Some(Arc::new(move || {
        let _ = progress_handle.emit("eddy:progress", ());
    })));
}
