// Thin Tauri shim over `rezon_core::embed`. Resolves app config dir
// for last-model persistence and emits `embed-loading` /
// `embed-loaded` / `embed-load-error` events to the frontend.

use std::path::PathBuf;
use std::sync::Arc;

use rezon_core::embed;
use rezon_core::search::SearchState;
use tauri::{AppHandle, Emitter, Manager, State};

pub use rezon_core::embed::{EmbedState, EmbedStatus};

fn config_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

pub fn read_last_embed_model(app: &AppHandle) -> Option<String> {
    config_dir(app).and_then(|d| embed::read_last_embed_model(&d))
}

fn persist_last_embed_model(app: &AppHandle, path: &str) {
    if let Some(dir) = config_dir(app) {
        embed::persist_last_embed_model(&dir, path);
    }
}

pub async fn do_load_embed(app: &AppHandle, path: String) -> Result<EmbedStatus, String> {
    let _ = app.emit("embed-loading", &path);
    let embed_state = app.state::<Arc<EmbedState>>().inner().clone();
    let search_state = app.state::<Arc<SearchState>>().inner().clone();
    let status = embed_state.load(path.clone()).await?;
    persist_last_embed_model(app, &path);
    embed::ensure_catchup_started(embed_state, search_state);
    let _ = app.emit("embed-loaded", &status);
    Ok(status)
}

#[tauri::command]
pub fn embed_status(state: State<'_, Arc<EmbedState>>) -> EmbedStatus {
    state.status()
}

#[tauri::command]
pub async fn embed_load_model(app: AppHandle, path: String) -> Result<EmbedStatus, String> {
    do_load_embed(&app, path).await
}

#[tauri::command]
pub fn vault_search_semantic(
    embed_state: State<'_, Arc<EmbedState>>,
    search_state: State<'_, Arc<SearchState>>,
    vault: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<rezon_core::search::SearchHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let lim = limit.unwrap_or(20).clamp(1, 200) as usize;
    embed::semantic_query(embed_state.as_ref(), search_state.as_ref(), &vault, q, lim)
}

/// Re-entry from `search::vault_index_touch` (and via `lib.rs`
/// auto-load handlers) when a save lands.
pub fn wake_catchup(app: &AppHandle) {
    app.state::<Arc<EmbedState>>().wake_catchup();
}
