// Thin Tauri command wrappers around `rezon_core::search`. The shell
// resolves `app_data_dir` once at startup (see `lib.rs::setup`) and
// hands it to `SearchState::new`. After that everything goes through
// the core implementation.

use std::sync::Arc;

use rezon_core::search;
use tauri::{AppHandle, State};

pub use rezon_core::search::{register_sqlite_vec, RelatedHit, SearchHit, SearchState};

#[tauri::command]
pub fn vault_search(
    state: State<'_, Arc<SearchState>>,
    vault: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, String> {
    let lim = limit.unwrap_or(50) as usize;
    search::vault_search_impl(state.as_ref(), &vault, &query, lim)
}

#[tauri::command]
pub fn vault_index_open(
    state: State<'_, Arc<SearchState>>,
    vault: String,
) -> Result<(), String> {
    search::vault_index_open(state.as_ref(), &vault)
}

#[tauri::command]
pub fn vault_related(
    state: State<'_, Arc<SearchState>>,
    vault: String,
    path: String,
    limit: Option<u32>,
) -> Result<Vec<RelatedHit>, String> {
    search::vault_related(state.as_ref(), &vault, &path, limit.unwrap_or(8))
}

#[tauri::command]
pub fn vault_index_touch(
    app: AppHandle,
    state: State<'_, Arc<SearchState>>,
    vault: String,
    path: String,
) -> Result<(), String> {
    search::vault_index_touch(state.as_ref(), &vault, &path)?;
    // Kick the embedder so it picks up the new dirty chunks. Lives
    // outside core because the embedder is still web-only (until P5c).
    crate::embed::wake_catchup(&app);
    Ok(())
}
