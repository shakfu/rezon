// Thin Tauri command wrappers over `rezon_core::vault`. The actual
// path-traversal-safe filesystem ops live in core; this module only
// adds the `#[tauri::command]` attribute and forwards arguments.
//
// `vault_write` additionally journals the change (pre-image +
// post-image snapshots under `<vault>/.rezon-history/`) and opts the
// vault into per-edit git versioning. The journal is best-effort:
// failures don't unwind the write but are surfaced via the returned
// optional warning so the UI can display them.

use std::path::Path;

use rezon_core::journal;
use rezon_core::vault;
use tauri::{AppHandle, Emitter};

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
pub fn vault_write(
    app: AppHandle,
    vault: String,
    path: String,
    content: String,
) -> Result<(), String> {
    // Capture the pre-image (if any) so the journal can offer
    // proper undo. Failure to read an existing file aborts the
    // write — better to error than to silently lose recoverable
    // state.
    let abs = Path::new(&path);
    let before: Option<Vec<u8>> = if abs.exists() {
        Some(std::fs::read(abs).map_err(|e| format!("read pre-image {path}: {e}"))?)
    } else {
        None
    };
    let after_bytes = content.clone().into_bytes();
    vault::vault_write(vault.clone(), path.clone(), content)?;
    // The frontend gets back the same `()` it always did, but
    // journal warnings (e.g. a pre-commit hook rejected our
    // auto-commit) now surface via a `vault-warning` Tauri event
    // so the UI can show a toast. The write itself already
    // succeeded — this is a notice, not an error.
    let rel = path
        .strip_prefix(&vault)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| path.clone());
    match journal::record_write(&vault, "manual_edit", &rel, before.as_deref(), Some(&after_bytes)) {
        Ok(out) => {
            if let Some(w) = out.git_warning {
                let _ = app.emit("vault-warning", format!("git: {w}"));
            }
        }
        Err(e) => {
            let _ = app.emit("vault-warning", format!("journal: {e}"));
        }
    }
    Ok(())
}

/// Undo the most-recent journaled change to the vault. Returns
/// `(path, target_id)` describing what was reverted, or an error
/// when there's nothing to undo.
#[tauri::command]
pub fn vault_undo(app: AppHandle, vault: String) -> Result<UndoReport, String> {
    let target = journal::last_undoable(&vault)?
        .ok_or_else(|| "nothing to undo".to_string())?;
    let (target_id, target_path, before_sha) = match target.op {
        journal::Op::Write { before_sha, .. } => (target.id, target.path, before_sha),
        _ => return Err("non-reversible journal entry".into()),
    };
    let abs = Path::new(&vault).join(&target_path);
    let current = std::fs::read(&abs).ok();
    match before_sha {
        Some(sha) => {
            let bytes = journal::read_blob(&vault, &sha)?;
            std::fs::write(&abs, &bytes)
                .map_err(|e| format!("restore {}: {e}", abs.display()))?;
        }
        None => {
            if abs.exists() {
                std::fs::remove_file(&abs)
                    .map_err(|e| format!("delete {}: {e}", abs.display()))?;
            }
        }
    }
    let new_after = std::fs::read(&abs).ok();
    match journal::record_undo(
        &vault,
        &target_path,
        &target_id,
        current.as_deref(),
        new_after.as_deref(),
    ) {
        Ok(out) => {
            if let Some(w) = out.git_warning {
                let _ = app.emit("vault-warning", format!("git: {w}"));
            }
        }
        Err(e) => {
            let _ = app.emit("vault-warning", format!("journal: {e}"));
        }
    }
    Ok(UndoReport {
        path: target_path,
        target_id,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub path: String,
    pub target_id: String,
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
