// Full-text search index for vault notes.
//
// One SQLite database per vault, stored under the app's data dir
// keyed by a hash of the vault path. The schema is:
//
//   files(path PRIMARY KEY, mtime, size)   -- change detection
//   notes USING fts5(path UNINDEXED, content, tokenize='porter unicode61')
//
// The `files` table lets us decide whether a file's FTS row is stale
// without keeping content in two places: we re-read from disk and
// re-tokenize only when (mtime, size) differs from the recorded row.
//
// A `notify` watcher per vault runs on its own OS thread and reapplies
// changes incrementally. The state holds a Weak<> back into the
// VaultIndex so dropping the state cleanly stops the thread.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct SearchState {
    inner: Mutex<HashMap<String, Arc<Mutex<VaultIndex>>>>,
}

impl SearchState {
    pub fn shutdown(&self) {
        let mut guard = self.inner.lock().unwrap();
        for (_, v) in guard.drain() {
            // Signal each watcher to exit and drop its watcher handle.
            if let Ok(mut idx) = v.lock() {
                idx.stop_watcher();
            }
        }
    }
}

struct VaultIndex {
    vault: PathBuf,
    db: Connection,
    // Held to keep the watcher alive. Dropping it stops notifications.
    _watcher: Option<RecommendedWatcher>,
    // Signal the worker thread to exit when the state is dropped.
    stop: Option<Sender<()>>,
}

impl VaultIndex {
    fn stop_watcher(&mut self) {
        // Dropping the watcher first stops further notify events; then
        // we close the worker channel by dropping the sender.
        self._watcher.take();
        self.stop.take();
    }
}

#[derive(Serialize)]
pub struct SearchHit {
    pub path: String,
    pub snippet: String,
}

fn vault_db_path(app: &AppHandle, vault: &Path) -> Result<PathBuf, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    let key = {
        let mut h = Sha256::new();
        h.update(vault.to_string_lossy().as_bytes());
        let digest = h.finalize();
        let hex: String = digest.iter().take(8).map(|b| format!("{:02x}", b)).collect();
        hex
    };
    fs::create_dir_all(data.join("vaults"))
        .map_err(|e| format!("create app data dir: {e}"))?;
    Ok(data.join("vaults").join(format!("{key}.db")))
}

fn init_schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS files (
            path  TEXT PRIMARY KEY,
            mtime INTEGER NOT NULL,
            size  INTEGER NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS notes USING fts5(
            path UNINDEXED,
            content,
            tokenize='porter unicode61'
         );",
    )
}

fn is_markdown(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lo = e.to_lowercase();
            lo == "md" || lo == "markdown"
        })
        .unwrap_or(false)
}

fn walk_markdown(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    for ent in read.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            walk_markdown(&path, out);
        } else if ft.is_file() && is_markdown(&path) {
            out.push(path);
        }
    }
}

fn file_meta(p: &Path) -> Option<(i64, i64)> {
    let md = fs::metadata(p).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = md.len() as i64;
    Some((mtime, size))
}

fn upsert_file(db: &Connection, path: &Path) -> rusqlite::Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let (mtime, size) = match file_meta(path) {
        Some(v) => v,
        None => return Ok(()),
    };

    // Skip if (mtime, size) match the recorded row.
    let row: Option<(i64, i64)> = db
        .query_row(
            "SELECT mtime, size FROM files WHERE path = ?1",
            params![path_str],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((m, s)) = row {
        if m == mtime && s == size {
            return Ok(());
        }
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    db.execute("DELETE FROM notes WHERE path = ?1", params![path_str])?;
    db.execute(
        "INSERT INTO notes (path, content) VALUES (?1, ?2)",
        params![path_str, content],
    )?;
    db.execute(
        "INSERT INTO files (path, mtime, size) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET mtime = excluded.mtime, size = excluded.size",
        params![path_str, mtime, size],
    )?;
    Ok(())
}

fn remove_file(db: &Connection, path: &Path) -> rusqlite::Result<()> {
    let path_str = path.to_string_lossy().to_string();
    db.execute("DELETE FROM notes WHERE path = ?1", params![path_str])?;
    db.execute("DELETE FROM files WHERE path = ?1", params![path_str])?;
    Ok(())
}

fn full_reindex(db: &Connection, vault: &Path) -> rusqlite::Result<()> {
    let mut found: Vec<PathBuf> = Vec::new();
    walk_markdown(vault, &mut found);

    // Upsert every present file.
    for p in &found {
        upsert_file(db, p)?;
    }

    // Delete rows for files that no longer exist.
    let present: std::collections::HashSet<String> = found
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let mut to_remove: Vec<String> = Vec::new();
    {
        let mut stmt = db.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows.flatten() {
            if !present.contains(&row) {
                to_remove.push(row);
            }
        }
    }
    for path_str in to_remove {
        db.execute("DELETE FROM notes WHERE path = ?1", params![path_str])?;
        db.execute("DELETE FROM files WHERE path = ?1", params![path_str])?;
    }
    Ok(())
}

fn open_or_create(app: &AppHandle, vault: &Path) -> Result<Arc<Mutex<VaultIndex>>, String> {
    let db_path = vault_db_path(app, vault)?;
    let db = Connection::open(&db_path).map_err(|e| format!("open db: {e}"))?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("WAL: {e}"))?;
    init_schema(&db).map_err(|e| format!("init schema: {e}"))?;
    full_reindex(&db, vault).map_err(|e| format!("reindex: {e}"))?;

    let idx = Arc::new(Mutex::new(VaultIndex {
        vault: vault.to_path_buf(),
        db,
        _watcher: None,
        stop: None,
    }));

    start_watcher(Arc::downgrade(&idx))?;
    Ok(idx)
}

fn start_watcher(weak: Weak<Mutex<VaultIndex>>) -> Result<(), String> {
    // Resolve the vault path once for the watcher; we need it to scope
    // notify and to compute relative event paths.
    let strong = match weak.upgrade() {
        Some(s) => s,
        None => return Ok(()),
    };
    let vault = strong.lock().unwrap().vault.clone();

    // notify uses sync channels; the worker thread drains them and
    // calls back into the index. A separate `stop` channel lets us
    // shut down deterministically when state is torn down.
    let (notify_tx, notify_rx) = channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = notify_tx.send(res);
    })
    .map_err(|e| format!("watcher: {e}"))?;
    watcher
        .watch(&vault, RecursiveMode::Recursive)
        .map_err(|e| format!("watch: {e}"))?;

    let (stop_tx, stop_rx) = channel::<()>();
    let weak_for_thread = weak.clone();
    thread::spawn(move || {
        loop {
            // Cheap shutdown check.
            if stop_rx.try_recv().is_ok() {
                break;
            }
            let evt = match notify_rx.recv_timeout(Duration::from_millis(300)) {
                Ok(e) => e,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            let Ok(event) = evt else { continue };
            let Some(strong) = weak_for_thread.upgrade() else {
                break;
            };
            let Ok(idx) = strong.lock() else { continue };
            apply_event(&idx, &event);
        }
    });

    // Store watcher + stop sender so dropping the index stops the thread.
    if let Some(strong) = weak.upgrade() {
        let mut g = strong.lock().unwrap();
        g._watcher = Some(watcher);
        g.stop = Some(stop_tx);
    }
    Ok(())
}

fn apply_event(idx: &VaultIndex, event: &Event) {
    let touched: Vec<&PathBuf> = event
        .paths
        .iter()
        .filter(|p| is_markdown(p) || matches!(event.kind, EventKind::Remove(_)))
        .collect();
    for p in touched {
        match event.kind {
            EventKind::Remove(_) => {
                let _ = remove_file(&idx.db, p);
            }
            _ => {
                if p.is_file() {
                    let _ = upsert_file(&idx.db, p);
                } else if !p.exists() {
                    let _ = remove_file(&idx.db, p);
                }
            }
        }
    }
}

fn get_or_open(
    app: &AppHandle,
    state: &SearchState,
    vault: &str,
) -> Result<Arc<Mutex<VaultIndex>>, String> {
    let mut map = state.inner.lock().unwrap();
    if let Some(v) = map.get(vault) {
        return Ok(v.clone());
    }
    let vp = PathBuf::from(vault);
    let idx = open_or_create(app, &vp)?;
    map.insert(vault.to_string(), idx.clone());
    Ok(idx)
}

#[tauri::command]
pub fn vault_search(
    app: AppHandle,
    state: tauri::State<'_, SearchState>,
    vault: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let lim = limit.unwrap_or(50).clamp(1, 500) as i64;

    let idx = get_or_open(&app, &state, &vault)?;
    let guard = idx.lock().map_err(|_| "search lock".to_string())?;
    // FTS5 MATCH wants its own query syntax. Quote each whitespace
    // token so user input like `foo bar` becomes `"foo" "bar"`, which
    // is implicit AND across phrases without interpreting special
    // characters. Empty tokens are skipped.
    let fts_query: String = q
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            let escaped = t.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect::<Vec<_>>()
        .join(" ");
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = guard
        .db
        .prepare(
            "SELECT path, snippet(notes, 1, '<<', '>>', '...', 12)
             FROM notes
             WHERE notes MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    let rows = stmt
        .query_map(params![fts_query, lim], |r| {
            Ok(SearchHit {
                path: r.get(0)?,
                snippet: r.get(1)?,
            })
        })
        .map_err(|e| format!("query: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        if let Ok(hit) = r {
            out.push(hit);
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn vault_index_open(
    app: AppHandle,
    state: tauri::State<'_, SearchState>,
    vault: String,
) -> Result<(), String> {
    // Force the index to be initialized for `vault`. The frontend
    // calls this when the user picks a vault so the watcher is
    // running before the first search.
    let _ = get_or_open(&app, &state, &vault)?;
    Ok(())
}

#[tauri::command]
pub fn vault_index_touch(
    app: AppHandle,
    state: tauri::State<'_, SearchState>,
    vault: String,
    path: String,
) -> Result<(), String> {
    // Called by the frontend after writing a file from within the app,
    // so the user sees fresh search results without waiting for the
    // notify event (which on macOS can lag).
    let idx = get_or_open(&app, &state, &vault)?;
    let guard = idx.lock().map_err(|_| "touch lock".to_string())?;
    let _ = upsert_file(&guard.db, Path::new(&path));
    Ok(())
}
