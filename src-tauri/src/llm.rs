use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{anyhow, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
// `Manager` is needed for app.path() and app.state()
use tokio::sync::Mutex;

const N_CTX: u32 = 4096;
const MAX_NEW_TOKENS: i32 = 1024;
const N_GPU_LAYERS: u32 = 999;

#[derive(Default)]
pub struct LlmState {
    backend: StdMutex<Option<Arc<LlamaBackend>>>,
    loaded: Mutex<Option<Loaded>>,
}

impl LlmState {
    fn ensure_backend(&self) -> Result<Arc<LlamaBackend>> {
        let mut guard = self.backend.lock().unwrap();
        if let Some(b) = guard.as_ref() {
            return Ok(b.clone());
        }
        let b = Arc::new(LlamaBackend::init().map_err(|e| anyhow!("backend init: {e}"))?);
        *guard = Some(b.clone());
        Ok(b)
    }

    pub fn shutdown(&self) {
        // Drop the model first (it owns llama_model pointers that talk to the
        // backend), then drop the backend (which frees ggml metal devices).
        // Doing this before the process calls exit() avoids racing C++ static
        // destructors inside ggml-metal.
        if let Ok(mut g) = self.loaded.try_lock() {
            *g = None;
        }
        if let Ok(mut g) = self.backend.lock() {
            *g = None;
        }
    }
}

struct Loaded {
    path: String,
    model: Arc<LlamaModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub loaded: bool,
    pub path: Option<String>,
}

fn config_file(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| anyhow!("app_config_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("mkdir {dir:?}: {e}"))?;
    Ok(dir.join("last_model.txt"))
}

fn persist_last_model(app: &AppHandle, path: &str) {
    match config_file(app) {
        Ok(p) => {
            if let Err(e) = std::fs::write(&p, path) {
                eprintln!("persist last_model to {p:?}: {e}");
            }
        }
        Err(e) => eprintln!("persist last_model: {e}"),
    }
}

pub fn read_last_model(app: &AppHandle) -> Option<String> {
    let p = config_file(app).ok()?;
    let s = std::fs::read_to_string(p).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub async fn do_load(app: &AppHandle, path: String) -> Result<ModelStatus, String> {
    let _ = app.emit("model-loading", &path);
    let state = app.state::<LlmState>();
    let backend = state.ensure_backend().map_err(|e| e.to_string())?;
    let path_for_load = path.clone();
    let model = tokio::task::spawn_blocking(move || -> Result<LlamaModel, String> {
        let params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
        LlamaModel::load_from_file(&backend, Path::new(&path_for_load), &params)
            .map_err(|e| format!("load_from_file: {e}"))
    })
    .await
    .map_err(|e| e.to_string())??;

    {
        let mut guard = state.loaded.lock().await;
        *guard = Some(Loaded {
            path: path.clone(),
            model: Arc::new(model),
        });
    }
    persist_last_model(app, &path);
    let status = ModelStatus {
        loaded: true,
        path: Some(path),
    };
    let _ = app.emit("model-loaded", &status);
    Ok(status)
}

#[tauri::command]
pub async fn load_model(app: AppHandle, path: String) -> Result<ModelStatus, String> {
    match do_load(&app, path).await {
        Ok(s) => Ok(s),
        Err(e) => {
            let _ = app.emit("model-load-error", &e);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn model_status(state: State<'_, LlmState>) -> Result<ModelStatus, String> {
    let guard = state.loaded.lock().await;
    Ok(match guard.as_ref() {
        Some(l) => ModelStatus {
            loaded: true,
            path: Some(l.path.clone()),
        },
        None => ModelStatus {
            loaded: false,
            path: None,
        },
    })
}

#[tauri::command]
pub async fn chat(app: AppHandle, messages: Vec<ChatMsg>) -> Result<String, String> {
    let (model, backend) = {
        let state = app.state::<LlmState>();
        let loaded = state.loaded.lock().await;
        let model = loaded
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?
            .model
            .clone();
        let backend = state
            .backend
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| "backend not initialized".to_string())?
            .clone();
        (model, backend)
    };

    let app_for_task = app.clone();
    tokio::task::spawn_blocking(move || run_chat(&app_for_task, &model, &backend, messages))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn run_chat(
    app: &AppHandle,
    model: &LlamaModel,
    backend: &LlamaBackend,
    messages: Vec<ChatMsg>,
) -> Result<String> {
    let chat_msgs: Vec<LlamaChatMessage> = messages
        .into_iter()
        .map(|m| {
            LlamaChatMessage::new(m.role, m.content)
                .map_err(|e| anyhow!("invalid chat message: {e}"))
        })
        .collect::<Result<_>>()?;

    let template = model
        .chat_template(None)
        .map_err(|e| anyhow!("model has no chat_template metadata: {e}"))?;
    let prompt = model
        .apply_chat_template(&template, &chat_msgs, true)
        .map_err(|e| anyhow!("apply_chat_template: {e}"))?;

    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(N_CTX));
    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| anyhow!("new_context: {e}"))?;

    let tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|e| anyhow!("str_to_token: {e}"))?;

    let prompt_len = tokens.len() as i32;
    let n_ctx_i = ctx.n_ctx() as i32;
    let max_new = MAX_NEW_TOKENS.min(n_ctx_i - prompt_len - 8).max(0);
    if max_new == 0 {
        return Err(anyhow!("prompt fills the context window"));
    }

    let mut batch = LlamaBatch::new(prompt_len.max(512) as usize, 1);
    let last_idx = prompt_len - 1;
    for (i, t) in tokens.iter().enumerate() {
        let i = i as i32;
        batch
            .add(*t, i, &[0], i == last_idx)
            .map_err(|e| anyhow!("batch.add prompt: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| anyhow!("decode prompt: {e}"))?;

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.7),
        LlamaSampler::dist(1234),
    ]);

    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut full = String::new();
    let mut n_cur = prompt_len;
    let mut produced = 0;

    while produced < max_new {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        let bytes = model
            .token_to_piece_bytes(token, 64, false, None)
            .map_err(|e| anyhow!("token_to_piece_bytes: {e}"))?;
        let mut piece = String::with_capacity(bytes.len() + 4);
        let _ = decoder.decode_to_string(&bytes, &mut piece, false);

        if !piece.is_empty() {
            full.push_str(&piece);
            let _ = app.emit("chat-token", &piece);
        }

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| anyhow!("batch.add gen: {e}"))?;
        n_cur += 1;
        produced += 1;
        ctx.decode(&mut batch)
            .map_err(|e| anyhow!("decode gen: {e}"))?;
    }

    let _ = app.emit("chat-done", &full);
    Ok(full)
}
