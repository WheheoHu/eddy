//! Tauri commands: thin wrappers over the engine. Sync engine calls (SQLite,
//! tantivy, settings) run through `blocking` so they never sit on the
//! webview's IPC thread; async engine calls are awaited directly. All errors
//! surface as their engine text, which is already user-facing (the engine
//! writes user messages in Chinese).

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use eddy_ai::AiSettings;
use eddy_core::{AiRole, BatchCallback, Engine, Result as EngineResult};

use crate::bridge::EngineState;
use crate::views::*;

fn engine(state: &State<'_, EngineState>) -> Engine {
    state.engine.clone()
}

/// Runs a sync engine call off the IPC thread and flattens the two error
/// layers (JoinError + EngineError) into one string.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> EngineResult<T> + Send + 'static,
{
    let inner = tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?;
    inner.map_err(|e| e.to_string())
}

// ---------- status ----------

#[tauri::command]
pub async fn status(state: State<'_, EngineState>) -> Result<StatusView, String> {
    Ok(engine(&state).status().map_err(|e| e.to_string())?.into())
}

#[tauri::command]
pub async fn pdf_available(state: State<'_, EngineState>) -> Result<bool, String> {
    Ok(state.pdfium)
}

// ---------- sidebar ----------

#[tauri::command]
pub async fn sources(state: State<'_, EngineState>) -> Result<Vec<SourceView>, String> {
    let engine = engine(&state);
    blocking(move || {
        engine
            .sources()
            .map(|list| list.into_iter().map(Into::into).collect())
    })
    .await
}

#[tauri::command]
pub async fn tags(state: State<'_, EngineState>) -> Result<Vec<TagView>, String> {
    let engine = engine(&state);
    blocking(move || {
        engine.tags().map(|list| {
            list.into_iter()
                .map(|t| TagView { id: t.id, name: t.name })
                .collect()
        })
    })
    .await
}

// ---------- lists ----------

#[tauri::command]
pub async fn items(
    state: State<'_, EngineState>,
    query: QueryView,
) -> Result<Vec<ItemRowView>, String> {
    let engine = engine(&state);
    let q = query.into();
    blocking(move || {
        engine
            .items(&q)
            .map(|rows| rows.into_iter().map(Into::into).collect())
    })
    .await
}

#[tauri::command]
pub async fn item_count(state: State<'_, EngineState>, query: QueryView) -> Result<u32, String> {
    let engine = engine(&state);
    let q = query.into();
    blocking(move || engine.item_count(&q)).await
}

#[tauri::command]
pub async fn item(state: State<'_, EngineState>, id: String) -> Result<Option<ItemRowView>, String> {
    let engine = engine(&state);
    blocking(move || engine.item(&id).map(|r| r.map(Into::into))).await
}

// ---------- reading ----------

#[tauri::command]
pub async fn document(
    state: State<'_, EngineState>,
    id: String,
) -> Result<Option<DocumentView>, String> {
    engine(&state)
        .document(&id)
        .await
        .map(|d| d.map(Into::into))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fetch_full_text(
    state: State<'_, EngineState>,
    id: String,
) -> Result<DocumentView, String> {
    engine(&state)
        .fetch_full_text(&id)
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_rules(state: State<'_, EngineState>, id: String) -> Result<(), String> {
    engine(&state)
        .apply_rules(&id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Every article image resolves through the engine (blob-cached offline);
/// returned as a data URL so the CSP never allows remote origins.
#[tauri::command]
pub async fn image(state: State<'_, EngineState>, url: String) -> Result<String, String> {
    let (bytes, mime) = engine(&state).image(&url).await.map_err(|e| e.to_string())?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub async fn set_read(state: State<'_, EngineState>, id: String, read: bool) -> Result<(), String> {
    engine(&state)
        .set_read(&id, read)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_starred(
    state: State<'_, EngineState>,
    id: String,
    starred: bool,
) -> Result<(), String> {
    engine(&state)
        .set_starred(&id, starred)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mark_all_read(state: State<'_, EngineState>, query: QueryView) -> Result<u32, String> {
    let engine = engine(&state);
    let query: eddy_core::ItemQuery = query.into();
    blocking(move || engine.mark_all_read(&query.filter, eddy_model::now_ms())).await
}

#[tauri::command]
pub async fn resume_point(
    state: State<'_, EngineState>,
    id: String,
) -> Result<Option<ResumePointView>, String> {
    let engine = engine(&state);
    blocking(move || engine.resume_point(&id).map(|r| r.map(Into::into))).await
}

#[tauri::command]
pub async fn record_progress(
    state: State<'_, EngineState>,
    id: String,
    block_id: String,
    offset: u32,
    at_end: bool,
) -> Result<(), String> {
    let engine = engine(&state);
    blocking(move || engine.record_progress(&id, &block_id, offset, at_end, false).map(|_| ())).await
}

// ---------- save / import / search / refresh ----------

#[tauri::command]
pub async fn save_url(state: State<'_, EngineState>, url: String) -> Result<ItemRowView, String> {
    engine(&state).save_url(&url).await.map(Into::into).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_file(
    state: State<'_, EngineState>,
    path: String,
) -> Result<ImportedView, String> {
    engine(&state)
        .import_file(std::path::Path::new(&path))
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_all(
    state: State<'_, EngineState>,
    query: String,
    limit: u32,
) -> Result<Vec<SearchResultView>, String> {
    engine(&state)
        .search_all(&query, limit)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn refresh(state: State<'_, EngineState>) -> Result<RefreshSummaryView, String> {
    engine(&state)
        .refresh()
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

// ---------- accounts / server ----------

#[tauri::command]
pub async fn configure_freshrss(
    state: State<'_, EngineState>,
    url: String,
    username: String,
    password: String,
) -> Result<eddy_model::Account, String> {
    engine(&state)
        .configure_freshrss(&url, &username, &password)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_account(state: State<'_, EngineState>) -> Result<(), String> {
    let engine = engine(&state);
    blocking(move || engine.remove_account()).await
}

#[tauri::command]
pub async fn connect_server(
    state: State<'_, EngineState>,
    url: String,
    code: String,
) -> Result<ServerStatusView, String> {
    engine(&state)
        .connect_server(&url, &code, "Eddy on Windows")
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disconnect_server(state: State<'_, EngineState>) -> Result<(), String> {
    engine(&state).disconnect_server().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn server_status(
    state: State<'_, EngineState>,
) -> Result<Option<ServerStatusView>, String> {
    engine(&state)
        .server_status()
        .await
        .map(|s| s.map(Into::into))
        .map_err(|e| e.to_string())
}

// ---------- AI ----------

#[tauri::command]
pub async fn ai_settings(state: State<'_, EngineState>) -> Result<AiSettings, String> {
    let engine = engine(&state);
    blocking(move || engine.ai_settings()).await
}

#[tauri::command]
pub async fn set_ai_settings(
    state: State<'_, EngineState>,
    settings: AiSettings,
) -> Result<(), String> {
    let engine = engine(&state);
    blocking(move || engine.set_ai_settings(settings)).await
}

#[tauri::command]
pub async fn set_ai_key(
    state: State<'_, EngineState>,
    role: String,
    key: Option<String>,
) -> Result<(), String> {
    let role = match role.as_str() {
        "translation" => AiRole::Translation,
        _ => AiRole::Summary,
    };
    engine(&state)
        .set_ai_role_key(role, key.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn has_ai_key(state: State<'_, EngineState>, role: String) -> Result<bool, String> {
    let role = match role.as_str() {
        "translation" => AiRole::Translation,
        _ => AiRole::Summary,
    };
    Ok(engine(&state).has_ai_role_key(role))
}

#[tauri::command]
pub async fn translate(
    app: AppHandle,
    state: State<'_, EngineState>,
    item_id: String,
    force: bool,
) -> Result<TranslationView, String> {
    let engine = engine(&state);
    let item_for_events = item_id.clone();
    let handle = app.clone();
    let on_batch: BatchCallback = Arc::new(move |segments: &HashMap<String, String>| {
        let _ = handle.emit(
            "eddy:translation",
            TranslationBatchView {
                item_id: item_for_events.clone(),
                segments: segments.clone(),
            },
        );
    });
    engine
        .translate(&item_id, force, Some(on_batch))
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_translation(
    state: State<'_, EngineState>,
    item_id: String,
) -> Result<(), String> {
    engine(&state).cancel_translation(&item_id);
    Ok(())
}

#[tauri::command]
pub async fn cached_translation(
    state: State<'_, EngineState>,
    item_id: String,
) -> Result<Option<TranslationView>, String> {
    let engine = engine(&state);
    blocking(move || engine.cached_translation(&item_id).map(|t| t.map(Into::into))).await
}

#[tauri::command]
pub async fn summarize(
    state: State<'_, EngineState>,
    item_id: String,
    force: bool,
) -> Result<SummaryView, String> {
    let result = engine(&state)
        .summarize(&item_id, force)
        .await
        .map_err(|e| e.to_string())?;
    Ok(SummaryView {
        tldr: result.summary.tldr,
        points: result.summary.points,
        cached: result.cached,
    })
}

#[tauri::command]
pub async fn cached_summary(
    state: State<'_, EngineState>,
    item_id: String,
) -> Result<Option<SummaryView>, String> {
    let engine = engine(&state);
    blocking(move || {
        engine.cached_summary(&item_id).map(|s| {
            s.map(|s| SummaryView {
                tldr: s.tldr,
                points: s.points,
                cached: true,
            })
        })
    })
    .await
}

// ---------- storage ----------

#[tauri::command]
pub async fn cache_usage(state: State<'_, EngineState>) -> Result<u64, String> {
    let engine = engine(&state);
    blocking(move || engine.cache_usage()).await
}

#[tauri::command]
pub async fn set_cache_limit(state: State<'_, EngineState>, bytes: u64) -> Result<(), String> {
    let engine = engine(&state);
    blocking(move || engine.set_cache_limit(bytes)).await
}

// ---------- misc ----------

#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
