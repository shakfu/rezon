// Local embedding model, kept in its own worker thread so the
// LlamaContext (which is not Send-safe to share) lives on one OS
// thread. The chat-side model in llm.rs has a similar worker; we
// don't share state with it so a user can pick separate GGUFs for
// chat and embedding (e.g. a 7B chat model + bge-small for embeds).
//
// The state exposes `embed(text)` which sends a request to the worker
// and awaits a oneshot response. It also drives a background catch-up
// loop that drains dirty chunks for any open vault and writes the
// resulting vectors back to vec_chunks.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::llm::LlmState;
use crate::search::{open_vault, SearchHit, SearchState};

const EMBED_N_CTX: u32 = 4096;
const N_GPU_LAYERS: u32 = 999;
const BATCH_LIMIT: usize = 16;

#[derive(Default)]
pub struct EmbedState {
    inner: Mutex<Option<Handle>>,
    // Wakes the catch-up loop. Set whenever a save lands so the worker
    // drains dirty rows for the affected vault.
    wake: Mutex<Option<mpsc::Sender<()>>>,
    // True once a catch-up loop has been spawned. Single-shot.
    catchup_started: AtomicBool,
}

struct Handle {
    path: String,
    dim: usize,
    sender: Option<mpsc::Sender<Request>>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.sender.take(); // close channel
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

enum Request {
    Embed {
        text: String,
        respond: mpsc::Sender<Result<Vec<f32>, String>>,
    },
}

#[derive(Serialize, Clone)]
pub struct EmbedStatus {
    pub loaded: bool,
    pub path: Option<String>,
    pub dim: Option<usize>,
}

impl EmbedState {
    pub fn status(&self) -> EmbedStatus {
        let g = self.inner.lock().unwrap();
        match g.as_ref() {
            Some(h) => EmbedStatus {
                loaded: true,
                path: Some(h.path.clone()),
                dim: Some(h.dim),
            },
            None => EmbedStatus {
                loaded: false,
                path: None,
                dim: None,
            },
        }
    }

    pub fn shutdown(&self) {
        if let Ok(mut g) = self.inner.lock() {
            *g = None;
        }
    }

    fn sender(&self) -> Option<mpsc::Sender<Request>> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref()?.sender.as_ref().cloned())
    }

    /// Send a text to the worker and block (briefly) on the response.
    /// Used by the catch-up loop from its own OS thread.
    fn embed_sync(&self, text: String) -> Result<Vec<f32>, String> {
        let s = self.sender().ok_or_else(|| "embed model not loaded".to_string())?;
        let (tx, rx) = mpsc::channel();
        s.send(Request::Embed { text, respond: tx })
            .map_err(|_| "embed worker exited".to_string())?;
        rx.recv().map_err(|_| "embed worker dropped response".to_string())?
    }
}

pub fn read_last_embed_model(app: &AppHandle) -> Option<String> {
    let p = app.path().app_config_dir().ok()?.join("last_embed_model.txt");
    let s = std::fs::read_to_string(p).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn persist_last_embed_model(app: &AppHandle, path: &str) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("last_embed_model.txt"), path);
    }
}

#[tauri::command]
pub fn embed_status(state: tauri::State<'_, EmbedState>) -> EmbedStatus {
    state.status()
}

#[tauri::command]
pub async fn embed_load_model(app: AppHandle, path: String) -> Result<EmbedStatus, String> {
    do_load_embed(&app, path).await
}

pub async fn do_load_embed(app: &AppHandle, path: String) -> Result<EmbedStatus, String> {
    let _ = app.emit("embed-loading", &path);
    let llm = app.state::<LlmState>();
    // Reuse the chat side's LlamaBackend so both models share one
    // global init (the backend is process-wide singleton in llama.cpp).
    let backend = ensure_backend(&llm).map_err(|e| e.to_string())?;

    let path_clone = path.clone();
    let backend_clone = backend.clone();
    let model: Arc<LlamaModel> = tokio::task::spawn_blocking(move || -> Result<LlamaModel, String> {
        let params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
        LlamaModel::load_from_file(&backend_clone, Path::new(&path_clone), &params)
            .map_err(|e| format!("load_from_file: {e}"))
    })
    .await
    .map_err(|e| e.to_string())??
    .into();

    let dim = model.n_embd() as usize;
    let (sender, join) = spawn_worker(model, backend);

    let state = app.state::<EmbedState>();
    {
        let mut g = state.inner.lock().unwrap();
        // Drop previous handle: closes channel, joins thread.
        *g = Some(Handle {
            path: path.clone(),
            dim,
            sender: Some(sender),
            join: Some(join),
        });
    }
    persist_last_embed_model(app, &path);
    ensure_catchup_started(app.clone());
    let status = state.status();
    let _ = app.emit("embed-loaded", &status);
    // Kick the worker so it starts catching up dirty chunks right away.
    if let Ok(g) = state.wake.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(());
        }
    }
    Ok(status)
}

fn ensure_backend(_llm: &LlmState) -> Result<Arc<LlamaBackend>> {
    // The chat side's LlmState owns its own backend but only exposes
    // it through private accessors; LlamaBackend::init is process-
    // global so a second call returns the same underlying state.
    Ok(Arc::new(
        LlamaBackend::init().map_err(|e| anyhow!("backend init: {e}"))?,
    ))
}

fn spawn_worker(
    model: Arc<LlamaModel>,
    backend: Arc<LlamaBackend>,
) -> (mpsc::Sender<Request>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Request>();
    let join = thread::spawn(move || worker_loop(model, backend, rx));
    (tx, join)
}

fn worker_loop(
    model: Arc<LlamaModel>,
    backend: Arc<LlamaBackend>,
    rx: mpsc::Receiver<Request>,
) {
    let model_ref: &LlamaModel = &model;
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(EMBED_N_CTX))
        .with_embeddings(true)
        .with_pooling_type(LlamaPoolingType::Mean);
    let mut ctx = match model_ref.new_context(&backend, ctx_params) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("embed new_context: {e}");
            while let Ok(req) = rx.recv() {
                match req {
                    Request::Embed { respond, .. } => {
                        let _ = respond.send(Err(format!("new_context: {e}")));
                    }
                }
            }
            return;
        }
    };

    while let Ok(req) = rx.recv() {
        match req {
            Request::Embed { text, respond } => {
                let result = embed_one(model_ref, &mut ctx, &text);
                let _ = respond.send(result);
            }
        }
    }
}

fn embed_one(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    text: &str,
) -> Result<Vec<f32>, String> {
    let n_embd = model.n_embd() as usize;
    let n_ctx = ctx.n_ctx() as usize;

    let mut tokens = model
        .str_to_token(text, AddBos::Always)
        .map_err(|e| format!("str_to_token: {e}"))?;
    if tokens.is_empty() {
        return Ok(vec![0.0; n_embd]);
    }
    if tokens.len() > n_ctx - 4 {
        tokens.truncate(n_ctx - 4);
    }

    // Each embed call starts from an empty KV cache so seq 0 is fresh.
    let _ = ctx.clear_kv_cache_seq(Some(0), None, None);

    let mut batch = LlamaBatch::new(tokens.len().max(512), 1);
    let last = tokens.len() - 1;
    for (i, t) in tokens.iter().enumerate() {
        // logits flag must be true for the last position of each seq
        // so the pooled embedding row is produced.
        batch
            .add(*t, i as i32, &[0], i == last)
            .map_err(|e| format!("batch.add: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("decode: {e}"))?;

    let emb = ctx
        .embeddings_seq_ith(0)
        .map_err(|e| format!("embeddings_seq_ith: {e}"))?;
    let mut v: Vec<f32> = emb.to_vec();
    // L2-normalize so cosine ≈ inner product and KNN behaves cleanly.
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    Ok(v)
}

// ---- Catch-up loop ---------------------------------------------------
//
// One process-global thread. Wakes on a channel signal, then iterates
// every currently-open vault in SearchState and drains BATCH_LIMIT
// dirty chunks per vault per pass, embedding and writing them back.
// Loops until no vault has dirty rows, then sleeps until the next
// wake. The frontend can call `vault_index_touch` after a write; the
// touch routes through this state's wake() (added below).

fn ensure_catchup_started(app: AppHandle) {
    let state = app.state::<EmbedState>();
    if state
        .catchup_started
        .swap(true, Ordering::Relaxed)
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<()>();
    {
        let mut g = state.wake.lock().unwrap();
        *g = Some(tx);
    }
    thread::spawn(move || catchup_loop(app, rx));
}

fn catchup_loop(app: AppHandle, rx: mpsc::Receiver<()>) {
    loop {
        // Drain any pile-up signals.
        let _ = rx.recv_timeout(Duration::from_secs(60));
        while let Ok(()) = rx.try_recv() {}
        let _ = drain_all_vaults(&app);
    }
}

fn drain_all_vaults(app: &AppHandle) -> Result<(), String> {
    let embed = app.state::<EmbedState>();
    let dim = match embed.status().dim {
        Some(d) => d,
        None => return Ok(()), // model not loaded
    };
    let search = app.state::<SearchState>();
    // Snapshot the current vault paths so we don't hold the state's
    // outer mutex while doing work.
    let paths: Vec<String> = {
        let map = search.inner_for_embed();
        map.keys().cloned().collect()
    };
    for vault in paths {
        let idx = match open_vault(app, &search, &vault) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Get up to BATCH_LIMIT dirty chunks per pass.
        let chunks = {
            let g = match idx.lock() {
                Ok(g) => g,
                Err(_) => continue,
            };
            // Make sure vec_chunks exists at this dim.
            if g.ensure_vec_table(dim).is_err() {
                continue;
            }
            match g.take_dirty_chunks(BATCH_LIMIT) {
                Ok(c) => c,
                Err(_) => continue,
            }
        };
        if chunks.is_empty() {
            continue;
        }
        let mut out: Vec<(i64, Vec<f32>)> = Vec::with_capacity(chunks.len());
        for c in chunks {
            match embed.embed_sync(c.text) {
                Ok(v) => out.push((c.id, v)),
                Err(_) => {
                    // If embedding fails, leave dirty=1 so we retry
                    // next pass. Don't spin: break to the sleep.
                    break;
                }
            }
        }
        if !out.is_empty() {
            if let Ok(g) = idx.lock() {
                let _ = g.write_embeddings(&out);
            }
            // If there might still be dirty rows, ping ourselves so
            // the next pass picks them up promptly.
            if let Ok(g) = embed.wake.lock() {
                if let Some(tx) = g.as_ref() {
                    let _ = tx.send(());
                }
            }
        }
    }
    Ok(())
}

// ---- Semantic search -------------------------------------------------

/// Embed a query string and run KNN against the vault's vec_chunks.
/// Returns an empty list when the embedding model isn't loaded or the
/// vault has no embedded chunks yet (the FE renders a hint in that
/// case).
pub fn semantic_query(
    app: &AppHandle,
    vault: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let embed = app.state::<EmbedState>();
    if !embed.status().loaded {
        return Ok(Vec::new());
    }
    let vec = embed.embed_sync(query.to_string())?;
    let search = app.state::<SearchState>();
    let idx = open_vault(app, &search, vault)?;
    let guard = idx.lock().map_err(|_| "semantic lock".to_string())?;
    guard.knn_search(&vec, limit)
}

#[tauri::command]
pub fn vault_search_semantic(
    app: AppHandle,
    vault: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let lim = limit.unwrap_or(20).clamp(1, 200) as usize;
    semantic_query(&app, &vault, q, lim)
}

/// Called from `vault_index_touch` so a save triggers a near-immediate
/// re-embed of the file's dirty chunks.
pub fn wake_catchup(app: &AppHandle) {
    let state = app.state::<EmbedState>();
    let tx_opt = match state.wake.lock() {
        Ok(g) => g.as_ref().cloned(),
        Err(_) => return,
    };
    if let Some(tx) = tx_opt {
        let _ = tx.send(());
    }
}
