// Thin Tauri command wrappers over `rezon_core::vault`. The actual
// path-traversal-safe filesystem ops live in core; this module only
// adds the `#[tauri::command]` attribute and forwards arguments.

use rezon_core::vault;

pub use rezon_core::vault::{ResolvedLink, VaultEntry};

#[tauri::command]
pub fn vault_list_tree(vault: String) -> Result<Vec<VaultEntry>, String> {
    vault::vault_list_tree(vault)
}

#[tauri::command]
pub fn vault_read(vault: String, path: String) -> Result<String, String> {
    vault::vault_read(vault, path)
}

#[tauri::command]
pub fn vault_write(vault: String, path: String, content: String) -> Result<(), String> {
    vault::vault_write(vault, path, content)
}

#[tauri::command]
pub fn vault_create(vault: String, path: String) -> Result<(), String> {
    vault::vault_create(vault, path)
}

#[tauri::command]
pub fn vault_mkdir(vault: String, path: String) -> Result<(), String> {
    vault::vault_mkdir(vault, path)
}

#[tauri::command]
pub fn vault_delete(vault: String, path: String) -> Result<(), String> {
    vault::vault_delete(vault, path)
}

#[tauri::command]
pub fn vault_rename(vault: String, from: String, to: String) -> Result<(), String> {
    vault::vault_rename(vault, from, to)
}

#[tauri::command]
pub fn vault_resolve_wikilink(
    vault: String,
    target: String,
    create_if_missing: bool,
) -> Result<ResolvedLink, String> {
    vault::vault_resolve_wikilink(vault, target, create_if_missing)
}
