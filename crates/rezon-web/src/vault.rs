// Vault commands: filesystem operations scoped to a user-chosen
// vault root. The frontend passes absolute paths; we validate that
// every path is contained inside the supplied vault root to prevent
// escape via "..".

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VaultEntry {
    File {
        name: String,
        path: String,
    },
    Dir {
        name: String,
        path: String,
        children: Vec<VaultEntry>,
    },
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn within(vault: &Path, target: &Path) -> Result<(), String> {
    let v = normalize(vault);
    let t = normalize(target);
    if !t.starts_with(&v) {
        return Err(format!(
            "path {:?} is outside vault {:?}",
            target, vault
        ));
    }
    Ok(())
}

fn read_tree(dir: &Path, vault: &Path) -> Result<Vec<VaultEntry>, String> {
    let mut entries: Vec<VaultEntry> = Vec::new();
    let read = fs::read_dir(dir).map_err(|e| format!("read_dir {:?}: {e}", dir))?;
    for ent in read.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = ent.path();
        let kind = ent.file_type().map_err(|e| e.to_string())?;
        let path_str = path.to_string_lossy().to_string();
        if kind.is_dir() {
            let children = read_tree(&path, vault)?;
            entries.push(VaultEntry::Dir {
                name,
                path: path_str,
                children,
            });
        } else if kind.is_file() {
            // Only surface markdown files in the tree. Other files are
            // ignored for now to keep the UI focused.
            let lower = name.to_lowercase();
            if lower.ends_with(".md") || lower.ends_with(".markdown") {
                entries.push(VaultEntry::File {
                    name,
                    path: path_str,
                });
            }
        }
    }
    entries.sort_by(|a, b| {
        let (ka, na) = match a {
            VaultEntry::Dir { name, .. } => (0u8, name.to_lowercase()),
            VaultEntry::File { name, .. } => (1u8, name.to_lowercase()),
        };
        let (kb, nb) = match b {
            VaultEntry::Dir { name, .. } => (0u8, name.to_lowercase()),
            VaultEntry::File { name, .. } => (1u8, name.to_lowercase()),
        };
        ka.cmp(&kb).then(na.cmp(&nb))
    });
    Ok(entries)
}

#[tauri::command]
pub fn vault_list_tree(vault: String) -> Result<Vec<VaultEntry>, String> {
    let root = PathBuf::from(&vault);
    if !root.is_dir() {
        return Err(format!("vault root is not a directory: {vault}"));
    }
    read_tree(&root, &root)
}

#[tauri::command]
pub fn vault_read(vault: String, path: String) -> Result<String, String> {
    let v = PathBuf::from(&vault);
    let p = PathBuf::from(&path);
    within(&v, &p)?;
    fs::read_to_string(&p).map_err(|e| format!("read {path}: {e}"))
}

#[tauri::command]
pub fn vault_write(vault: String, path: String, content: String) -> Result<(), String> {
    let v = PathBuf::from(&vault);
    let p = PathBuf::from(&path);
    within(&v, &p)?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {:?}: {e}", parent))?;
    }
    fs::write(&p, content).map_err(|e| format!("write {path}: {e}"))
}

#[tauri::command]
pub fn vault_create(vault: String, path: String) -> Result<(), String> {
    let v = PathBuf::from(&vault);
    let p = PathBuf::from(&path);
    within(&v, &p)?;
    if p.exists() {
        return Err(format!("already exists: {path}"));
    }
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {:?}: {e}", parent))?;
    }
    fs::write(&p, "").map_err(|e| format!("create {path}: {e}"))
}

#[tauri::command]
pub fn vault_mkdir(vault: String, path: String) -> Result<(), String> {
    let v = PathBuf::from(&vault);
    let p = PathBuf::from(&path);
    within(&v, &p)?;
    if p.exists() {
        return Err(format!("already exists: {path}"));
    }
    fs::create_dir_all(&p).map_err(|e| format!("mkdir {path}: {e}"))
}

#[tauri::command]
pub fn vault_delete(vault: String, path: String) -> Result<(), String> {
    let v = PathBuf::from(&vault);
    let p = PathBuf::from(&path);
    within(&v, &p)?;
    if p.is_dir() {
        fs::remove_dir_all(&p).map_err(|e| format!("rmdir {path}: {e}"))
    } else {
        fs::remove_file(&p).map_err(|e| format!("rm {path}: {e}"))
    }
}

#[tauri::command]
pub fn vault_rename(vault: String, from: String, to: String) -> Result<(), String> {
    let v = PathBuf::from(&vault);
    let a = PathBuf::from(&from);
    let b = PathBuf::from(&to);
    within(&v, &a)?;
    within(&v, &b)?;
    fs::rename(&a, &b).map_err(|e| format!("rename: {e}"))
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResolvedLink {
    pub path: String,
    pub created: bool,
}

// Resolve a wikilink target ([[foo]] or [[folder/foo]]) to an absolute
// path inside the vault. If `create_if_missing` is true and no match
// exists, create the file in the vault root with a ".md" extension.
//
// Resolution order:
//   1. exact relative path under vault root (with or without .md)
//   2. first file in the tree whose stem matches case-insensitively
#[tauri::command]
pub fn vault_resolve_wikilink(
    vault: String,
    target: String,
    create_if_missing: bool,
) -> Result<ResolvedLink, String> {
    let root = PathBuf::from(&vault);
    if !root.is_dir() {
        return Err("vault root is not a directory".into());
    }
    let mut t = target.trim().to_string();
    if t.is_empty() {
        return Err("empty target".into());
    }
    // Strip a leading "/" so callers don't accidentally escape root.
    while t.starts_with('/') {
        t.remove(0);
    }

    // 1. exact relative path
    let with_ext = if t.to_lowercase().ends_with(".md") {
        t.clone()
    } else {
        format!("{t}.md")
    };
    let direct = root.join(&with_ext);
    if direct.is_file() {
        within(&root, &direct)?;
        return Ok(ResolvedLink {
            path: direct.to_string_lossy().to_string(),
            created: false,
        });
    }

    // 2. recursive stem match (case-insensitive)
    let stem = Path::new(&t)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&t)
        .to_lowercase();
    if let Some(found) = find_by_stem(&root, &stem) {
        return Ok(ResolvedLink {
            path: found.to_string_lossy().to_string(),
            created: false,
        });
    }

    if !create_if_missing {
        return Err(format!("not found: {t}"));
    }

    let target_path = root.join(&with_ext);
    within(&root, &target_path)?;
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    fs::write(&target_path, "").map_err(|e| format!("create: {e}"))?;
    Ok(ResolvedLink {
        path: target_path.to_string_lossy().to_string(),
        created: true,
    })
}

fn find_by_stem(dir: &Path, stem_lower: &str) -> Option<PathBuf> {
    let read = fs::read_dir(dir).ok()?;
    for ent in read.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = ent.path();
        let kind = ent.file_type().ok()?;
        if kind.is_dir() {
            if let Some(hit) = find_by_stem(&path, stem_lower) {
                return Some(hit);
            }
        } else if kind.is_file() {
            let lower = name.to_lowercase();
            if !(lower.ends_with(".md") || lower.ends_with(".markdown")) {
                continue;
            }
            if let Some(s) = Path::new(&name).file_stem().and_then(|s| s.to_str()) {
                if s.to_lowercase() == stem_lower {
                    return Some(path);
                }
            }
        }
    }
    None
}
