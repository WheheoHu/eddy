#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Eddy for Windows: a Tauri 2 shell over the Rust core, mirroring the macOS
//! app's AppModel pattern — one engine, listeners that pulse the webview,
//! snapshot reloads on the frontend.

mod bridge;
mod commands;
mod platform;
mod views;

use std::sync::Arc;

use eddy_core::{Engine, Platform};

fn main() {
    let data_dir = bridge::resolve_data_dir();
    let (engine, pdfium) = match bridge::open_engine(&data_dir) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Eddy engine failed to open {data_dir:?}: {e}");
            std::process::exit(1);
        }
    };
    engine.set_pdf_reader(Some(Arc::new(eddy_pdf::PdfiumReader::new())));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(bridge::EngineState {
            engine: engine.clone(),
            data_dir: data_dir.clone(),
            pdfium,
        })
        .setup(move |app| {
            bridge::install_listeners(app.handle().clone(), &engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::pdf_available,
            commands::sources,
            commands::tags,
            commands::items,
            commands::item_count,
            commands::item,
            commands::document,
            commands::fetch_full_text,
            commands::apply_rules,
            commands::image,
            commands::set_read,
            commands::set_starred,
            commands::mark_all_read,
            commands::resume_point,
            commands::record_progress,
            commands::save_url,
            commands::import_file,
            commands::search_all,
            commands::refresh,
            commands::cache_usage,
            commands::set_cache_limit,
            commands::configure_freshrss,
            commands::remove_account,
            commands::connect_server,
            commands::disconnect_server,
            commands::server_status,
            commands::ai_settings,
            commands::set_ai_settings,
            commands::set_ai_key,
            commands::has_ai_key,
            commands::translate,
            commands::cancel_translation,
            commands::cached_translation,
            commands::summarize,
            commands::cached_summary,
            commands::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
