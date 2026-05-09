use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex as StdMutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionStreamOptions, CreateChatCompletionRequestArgs,
    },
    Client,
};
use futures::StreamExt;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::openai::OpenAIChatTemplateParams;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::agent::delta::{AgentDelta, FinishReason, StreamStats};

const N_CTX: u32 = 4096;
const MAX_NEW_TOKENS: i32 = 1024;
const N_GPU_LAYERS: u32 = 999;

#[derive(Default)]
pub struct LlmState {
    backend: StdMutex<Option<Arc<LlamaBackend>>>,
    loaded: StdMutex<Option<LoadedHandle>>,
    cancel: Arc<AtomicBool>,
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

    /// Submit a tool-aware streaming chat to the loaded local model's
    /// worker. Returns a receiver that yields `AgentDelta` values; the
    /// channel closes when generation finishes. Used by
    /// `agent::local::LocalProvider`.
    pub(crate) fn agent_chat_stream(
        &self,
        messages_json: String,
        tools_json: String,
        cancel: Arc<AtomicBool>,
    ) -> std::result::Result<UnboundedReceiver<std::result::Result<AgentDelta, String>>, String>
    {
        let sender = {
            let g = self.loaded.lock().unwrap();
            g.as_ref()
                .ok_or_else(|| "no model loaded".to_string())?
                .sender
                .as_ref()
                .ok_or_else(|| "model worker exited".to_string())?
                .clone()
        };
        let (tx, rx) = unbounded_channel();
        sender
            .send(WorkerRequest::AgentChat {
                messages_json,
                tools_json,
                cancel,
                deltas: tx,
            })
            .map_err(|_| "model worker exited".to_string())?;
        Ok(rx)
    }

    pub fn shutdown(&self) {
        // Order matters here: ggml-metal's process-exit destructor (run via
        // __cxa_finalize after main returns) asserts that no resource sets
        // are alive on the metal device. If the worker thread is still
        // holding a LlamaContext at that point, its KV-cache buffers are
        // registered against the device and the assert fires. So we must
        // (a) signal any in-flight chat to abort, (b) drop the loaded
        // handle and *join* the worker thread before returning, and only
        // then (c) drop the backend Arc.
        self.cancel.store(true, Ordering::Relaxed);
        if let Ok(mut g) = self.loaded.lock() {
            // Dropping the LoadedHandle closes the channel and joins the
            // worker thread (see Drop impl).
            *g = None;
        }
        if let Ok(mut g) = self.backend.lock() {
            *g = None;
        }
    }
}

// A handle to the per-model worker thread. Dropping closes the channel
// (waking the worker if it's idle on rx.recv) and then joins the thread,
// guaranteeing the LlamaContext + LlamaModel + the worker's Arc clone of
// LlamaBackend are all released before this drop returns.
struct LoadedHandle {
    path: String,
    sender: Option<mpsc::Sender<WorkerRequest>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for LoadedHandle {
    fn drop(&mut self) {
        // Step 1: drop the sender to close the channel. If the worker is
        // idle (blocked on rx.recv) it returns Err and the loop exits. If
        // the worker is mid-chat it will see the cancel flag (set by
        // LlmState::shutdown or by chat() before each new request) and
        // bail out of the generation loop, then loop back to rx.recv,
        // which now also returns Err.
        self.sender.take();
        // Step 2: wait for the worker thread to actually finish, so its
        // LlamaContext (and the metal buffers behind its KV cache) are
        // gone before this function returns.
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

enum WorkerRequest {
    Chat {
        messages: Vec<ChatMsg>,
        cancel: Arc<AtomicBool>,
        app: AppHandle,
        respond: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
    /// Tool-aware streaming chat. Driven by the agent loop. Emits
    /// `AgentDelta` values through `deltas`; closes the channel when
    /// generation completes (normally, due to cancel, or on error).
    AgentChat {
        messages_json: String,
        tools_json: String,
        cancel: Arc<AtomicBool>,
        deltas: UnboundedSender<std::result::Result<AgentDelta, String>>,
    },
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStats {
    pub provider: String,
    pub prompt_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub gen_tokens: u32,
    pub duration_ms: u64,
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
    let backend_for_load = backend.clone();
    let model = tokio::task::spawn_blocking(move || -> Result<LlamaModel, String> {
        let params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
        LlamaModel::load_from_file(&backend_for_load, Path::new(&path_for_load), &params)
            .map_err(|e| format!("load_from_file: {e}"))
    })
    .await
    .map_err(|e| e.to_string())??;

    let model = Arc::new(model);
    let (sender, join) = spawn_worker(model, backend);
    // Tell the previous worker (if any) to abort an in-flight chat so we
    // don't block here for up to MAX_NEW_TOKENS while it finishes. The
    // next chat() resets the flag to false before dispatching.
    state.cancel.store(true, Ordering::Relaxed);
    {
        let mut guard = state.loaded.lock().unwrap();
        // Replacing drops the previous LoadedHandle, whose Drop impl closes
        // the channel and joins the previous worker thread before this
        // assignment returns. So the previous model + context are fully
        // released before the new worker takes over.
        *guard = Some(LoadedHandle {
            path: path.clone(),
            sender: Some(sender),
            join: Some(join),
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
pub fn model_status(state: State<'_, LlmState>) -> Result<ModelStatus, String> {
    let guard = state.loaded.lock().unwrap();
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOpts {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

// Cloud provider catalog. Loaded from src-tauri/models.json at build
// time via include_str!, parsed once on first access. To add or change
// models, edit models.json — no code changes required.
//
// Field names mirror the JS-facing camelCase shape (see
// `CloudProviderInfo` below) so the same struct can deserialize the
// JSON and back the public command response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudProviderDef {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) env_var: String,
    pub(crate) base_url: String,
    pub(crate) default_model: String,
    pub(crate) recommended_models: Vec<String>,
    pub(crate) user_configurable: bool,
}

#[derive(Debug, Deserialize)]
struct ModelsConfig {
    providers: Vec<CloudProviderDef>,
}

const MODELS_JSON: &str = include_str!("../models.json");

fn cloud_providers_catalog() -> &'static [CloudProviderDef] {
    static CACHE: std::sync::OnceLock<Vec<CloudProviderDef>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let cfg: ModelsConfig =
            serde_json::from_str(MODELS_JSON).expect("models.json failed to parse at startup");
        cfg.providers
    })
}

pub(crate) fn cloud_provider_def(key: &str) -> Option<&'static CloudProviderDef> {
    cloud_providers_catalog().iter().find(|p| p.key == key)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProviderInfo {
    key: String,
    label: String,
    env_var: String,
    default_model: String,
    recommended_models: Vec<String>,
    api_key_set: bool,
    user_configurable: bool,
}

#[tauri::command]
pub fn cloud_providers() -> Vec<CloudProviderInfo> {
    cloud_providers_catalog()
        .iter()
        .map(|p| CloudProviderInfo {
            key: p.key.clone(),
            label: p.label.clone(),
            env_var: p.env_var.clone(),
            default_model: p.default_model.clone(),
            recommended_models: p.recommended_models.clone(),
            // Treat user-configurable providers as always available; the
            // user supplies the key (if any) at request time.
            api_key_set: p.user_configurable
                || std::env::var(&p.env_var)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            user_configurable: p.user_configurable,
        })
        .collect()
}

#[tauri::command]
pub fn cancel_chat(state: State<'_, LlmState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn chat(
    app: AppHandle,
    messages: Vec<ChatMsg>,
    opts: ChatOpts,
) -> Result<String, String> {
    let cancel = {
        let state = app.state::<LlmState>();
        state.cancel.store(false, Ordering::Relaxed);
        state.cancel.clone()
    };

    if opts.provider == "local" {
        return run_local_chat(&app, messages, cancel).await;
    }
    let def = cloud_provider_def(&opts.provider)
        .ok_or_else(|| format!("unknown provider: {}", opts.provider))?;

    let (api_key, base_url) = if def.user_configurable {
        let base_url = opts
            .base_url
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "base URL is required".to_string())?
            .to_string();
        // API key is optional for local OpenAI-compatible servers (Ollama,
        // llama.cpp server). async-openai sends an Authorization header
        // either way; servers that don't check it ignore the value.
        let api_key = opts
            .api_key
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "no-key".to_string());
        (api_key, base_url)
    } else {
        let api_key = std::env::var(&def.env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{} is not set", &def.env_var))?;
        (api_key, def.base_url.clone())
    };

    let model = opts
        .model
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            if def.default_model.is_empty() {
                None
            } else {
                Some(def.default_model.to_string())
            }
        })
        .ok_or_else(|| "model is required".to_string())?;

    run_cloud_chat(
        &app,
        messages,
        model,
        api_key,
        base_url,
        opts.provider.clone(),
        cancel,
    )
    .await
}

async fn run_local_chat(
    app: &AppHandle,
    messages: Vec<ChatMsg>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let sender = {
        let state = app.state::<LlmState>();
        let guard = state.loaded.lock().unwrap();
        guard
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?
            .sender
            .as_ref()
            .ok_or_else(|| "model worker exited".to_string())?
            .clone()
    };

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    sender
        .send(WorkerRequest::Chat {
            messages,
            cancel,
            app: app.clone(),
            respond: respond_tx,
        })
        .map_err(|_| "model worker exited".to_string())?;

    respond_rx
        .await
        .map_err(|_| "model worker dropped response".to_string())?
}

fn to_openai_messages(
    messages: Vec<ChatMsg>,
) -> Result<Vec<ChatCompletionRequestMessage>, String> {
    messages
        .into_iter()
        .map(|m| -> Result<ChatCompletionRequestMessage, String> {
            match m.role.as_str() {
                "system" => Ok(ChatCompletionRequestSystemMessageArgs::default()
                    .content(m.content)
                    .build()
                    .map_err(|e| format!("system msg: {e}"))?
                    .into()),
                "user" => Ok(ChatCompletionRequestUserMessageArgs::default()
                    .content(m.content)
                    .build()
                    .map_err(|e| format!("user msg: {e}"))?
                    .into()),
                "assistant" => Ok(ChatCompletionRequestAssistantMessageArgs::default()
                    .content(m.content)
                    .build()
                    .map_err(|e| format!("assistant msg: {e}"))?
                    .into()),
                other => Err(format!("unknown role: {other}")),
            }
        })
        .collect()
}

async fn run_cloud_chat(
    app: &AppHandle,
    messages: Vec<ChatMsg>,
    model: String,
    api_key: String,
    base_url: String,
    provider: String,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let started = Instant::now();
    let openai_msgs = to_openai_messages(messages)?;
    let request = CreateChatCompletionRequestArgs::default()
        .model(&model)
        .messages(openai_msgs)
        .stream_options(ChatCompletionStreamOptions {
            include_usage: Some(true),
            include_obfuscation: None,
        })
        .build()
        .map_err(|e| format!("build request: {e}"))?;

    let cfg = OpenAIConfig::new()
        .with_api_key(api_key)
        .with_api_base(base_url);
    let client = Client::with_config(cfg);

    let mut stream = client
        .chat()
        .create_stream(request)
        .await
        .map_err(|e| format!("create_stream: {e}"))?;

    let mut full = String::new();
    let mut prompt_tokens: Option<u32> = None;
    let mut gen_tokens: Option<u32> = None;
    while let Some(result) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match result {
            Ok(response) => {
                if let Some(usage) = response.usage.as_ref() {
                    prompt_tokens = Some(usage.prompt_tokens);
                    gen_tokens = Some(usage.completion_tokens);
                }
                for choice in response.choices {
                    if let Some(content) = choice.delta.content {
                        if !content.is_empty() {
                            full.push_str(&content);
                            let _ = app.emit("chat-token", &content);
                        }
                    }
                }
            }
            Err(e) => return Err(format!("stream: {e}")),
        }
    }

    let stats = ChatStats {
        provider,
        prompt_tokens,
        cached_tokens: None,
        // Fall back to a rough char-based estimate if the server didn't
        // return usage (e.g. some OpenAI-compatible servers ignore
        // `stream_options.include_usage`).
        gen_tokens: gen_tokens.unwrap_or_else(|| (full.len() as u32).div_ceil(4)),
        duration_ms: started.elapsed().as_millis() as u64,
    };
    let _ = app.emit("chat-stats", &stats);
    let _ = app.emit("chat-done", &full);
    Ok(full)
}

fn spawn_worker(
    model: Arc<LlamaModel>,
    backend: Arc<LlamaBackend>,
) -> (mpsc::Sender<WorkerRequest>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<WorkerRequest>();
    let join = std::thread::spawn(move || worker_loop(model, backend, rx));
    (tx, join)
}

fn worker_loop(
    model: Arc<LlamaModel>,
    backend: Arc<LlamaBackend>,
    rx: mpsc::Receiver<WorkerRequest>,
) {
    // Borrow the model for the lifetime of this stack frame; the LlamaContext
    // borrows from it. Both `ctx` and the borrow drop before `model` does
    // because of reverse-declaration drop order.
    let model_ref: &LlamaModel = &model;
    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(N_CTX));
    let mut ctx = match model_ref.new_context(&backend, ctx_params) {
        Ok(c) => c,
        Err(e) => {
            let err = format!("new_context: {e}");
            eprintln!("{err}");
            // Reply to any pending requests with the init error rather than
            // hanging.
            while let Ok(WorkerRequest::Chat { respond, .. }) = rx.recv() {
                let _ = respond.send(Err(err.clone()));
            }
            return;
        }
    };

    // Tokens currently committed to the KV cache for sequence 0. Used to
    // find the longest common prefix with each new prompt so we only have
    // to re-decode the divergent suffix.
    let mut cached: Vec<LlamaToken> = Vec::new();

    while let Ok(req) = rx.recv() {
        match req {
            WorkerRequest::Chat {
                messages,
                cancel,
                app,
                respond,
            } => {
                let result =
                    run_chat_with_cache(&app, model_ref, &mut ctx, &mut cached, messages, cancel);
                let _ = respond.send(result);
            }
            WorkerRequest::AgentChat {
                messages_json,
                tools_json,
                cancel,
                deltas,
            } => {
                // `deltas` is dropped at the end of this scope, closing
                // the channel and signaling completion to the consumer.
                let _ = run_agent_with_cache(
                    model_ref,
                    &mut ctx,
                    &mut cached,
                    &messages_json,
                    &tools_json,
                    cancel,
                    &deltas,
                );
            }
        }
    }
}

fn run_chat_with_cache(
    app: &AppHandle,
    model: &LlamaModel,
    ctx: &mut LlamaContext<'_>,
    cached: &mut Vec<LlamaToken>,
    messages: Vec<ChatMsg>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let started = Instant::now();
    let chat_msgs: Vec<LlamaChatMessage> = messages
        .into_iter()
        .map(|m| {
            LlamaChatMessage::new(m.role, m.content)
                .map_err(|e| format!("invalid chat message: {e}"))
        })
        .collect::<Result<_, _>>()?;

    let template = model
        .chat_template(None)
        .map_err(|e| format!("model has no chat_template metadata: {e}"))?;
    let prompt = model
        .apply_chat_template(&template, &chat_msgs, true)
        .map_err(|e| format!("apply_chat_template: {e}"))?;

    let new_tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|e| format!("str_to_token: {e}"))?;

    // Longest common prefix between the cached tokens (in the KV cache) and
    // the freshly tokenized prompt.
    let mut common = 0usize;
    while common < cached.len()
        && common < new_tokens.len()
        && cached[common] == new_tokens[common]
    {
        common += 1;
    }

    // Drop everything beyond the common prefix from the KV cache and from
    // our shadow vector.
    if common < cached.len() {
        ctx.clear_kv_cache_seq(Some(0), Some(common as u32), None)
            .map_err(|e| format!("clear_kv_cache_seq: {e}"))?;
        cached.truncate(common);
    }

    let to_add = &new_tokens[common..];
    let prompt_len = new_tokens.len() as i32;
    let n_ctx_i = ctx.n_ctx() as i32;
    let max_new = MAX_NEW_TOKENS.min(n_ctx_i - prompt_len - 8).max(0);
    if max_new == 0 {
        return Err("prompt fills the context window".to_string());
    }
    if to_add.is_empty() {
        // Nothing to feed: the new prompt is already entirely in the KV
        // cache. This shouldn't happen because chat templates append an
        // assistant-prefix marker each turn, but guard so we don't sample
        // off a stale logits row.
        return Err("no new tokens to decode".to_string());
    }

    // Decode the divergent suffix of the prompt into the KV cache.
    let mut batch = LlamaBatch::new(to_add.len().max(512), 1);
    let last_idx = to_add.len() - 1;
    for (i, t) in to_add.iter().enumerate() {
        let pos = common as i32 + i as i32;
        batch
            .add(*t, pos, &[0], i == last_idx)
            .map_err(|e| format!("batch.add prompt: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("decode prompt: {e}"))?;
    cached.extend_from_slice(to_add);

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.7),
        LlamaSampler::dist(1234),
    ]);

    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut full = String::new();
    let mut n_cur = prompt_len;
    let mut produced = 0;

    while produced < max_new {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            // Don't push the EOG token to `cached` — we already truncated
            // back to `common` and only added prompt tokens. The KV cache
            // currently reflects the prompt only, which is what we want
            // for the next turn's prefix match.
            break;
        }

        let bytes = model
            .token_to_piece_bytes(token, 64, false, None)
            .map_err(|e| format!("token_to_piece_bytes: {e}"))?;
        let mut piece = String::with_capacity(bytes.len() + 4);
        let _ = decoder.decode_to_string(&bytes, &mut piece, false);

        if !piece.is_empty() {
            full.push_str(&piece);
            let _ = app.emit("chat-token", &piece);
        }

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("batch.add gen: {e}"))?;
        n_cur += 1;
        produced += 1;
        ctx.decode(&mut batch)
            .map_err(|e| format!("decode gen: {e}"))?;
        // Track the generated token in the KV cache shadow, so the next
        // turn's prompt (which will include this assistant response) finds
        // a longer matching prefix.
        cached.push(token);
    }

    let stats = ChatStats {
        provider: "local".to_string(),
        prompt_tokens: Some(prompt_len as u32),
        cached_tokens: Some(common as u32),
        gen_tokens: produced as u32,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    let _ = app.emit("chat-stats", &stats);
    let _ = app.emit("chat-done", &full);
    Ok(full)
}

/// Tool-aware streaming chat against the loaded local model.
///
/// Mirrors `run_chat_with_cache` (KV-cache reuse, cancel polling,
/// stats accounting), but routes through llama-cpp-2's OpenAI-compat
/// surface so the model emits content + tool calls in OpenAI shape and
/// `ChatParseStateOaicompat` produces deltas the agent loop can
/// consume directly. Grammar-constrained sampling is intentionally
/// disabled — see `docs/dev/local_tool_calling.md` for the upstream
/// `GGML_ASSERT` bug that motivates skipping it on this version.
fn run_agent_with_cache(
    model: &LlamaModel,
    ctx: &mut LlamaContext<'_>,
    cached: &mut Vec<LlamaToken>,
    messages_json: &str,
    tools_json: &str,
    cancel: Arc<AtomicBool>,
    deltas: &UnboundedSender<std::result::Result<AgentDelta, String>>,
) -> std::result::Result<(), String> {
    let started = Instant::now();

    let template = model
        .chat_template(None)
        .map_err(|e| format!("chat_template: {e}"))?;

    let params = OpenAIChatTemplateParams {
        messages_json,
        tools_json: Some(tools_json),
        tool_choice: None,
        json_schema: None,
        // Skip grammar synthesis — see docs/dev/local_tool_calling.md.
        grammar: None,
        reasoning_format: None,
        chat_template_kwargs: None,
        add_generation_prompt: true,
        use_jinja: true,
        parallel_tool_calls: true,
        // Phase-4 simplification: disable thinking blocks. Re-enable
        // once the UI has a "Show reasoning" affordance.
        enable_thinking: false,
        // Match the existing chat path: tokenizer adds BOS, not the
        // template.
        add_bos: false,
        add_eos: false,
        parse_tool_calls: true,
    };
    let tmpl = model
        .apply_chat_template_oaicompat(&template, &params)
        .map_err(|e| format!("apply_chat_template_oaicompat: {e}"))?;

    if !tmpl.parse_tool_calls || tmpl.parser.is_none() {
        let msg = format!(
            "model's chat template does not declare tool support (parse_tool_calls={}, parser={})",
            tmpl.parse_tool_calls,
            tmpl.parser.is_some()
        );
        let _ = deltas.send(Err(msg.clone()));
        return Err(msg);
    }

    let prompt_tokens = model
        .str_to_token(&tmpl.prompt, AddBos::Always)
        .map_err(|e| format!("str_to_token: {e}"))?;

    let mut common = 0usize;
    while common < cached.len()
        && common < prompt_tokens.len()
        && cached[common] == prompt_tokens[common]
    {
        common += 1;
    }
    if common < cached.len() {
        ctx.clear_kv_cache_seq(Some(0), Some(common as u32), None)
            .map_err(|e| format!("clear_kv_cache_seq: {e}"))?;
        cached.truncate(common);
    }

    let to_add = &prompt_tokens[common..];
    let prompt_len = prompt_tokens.len() as i32;
    let n_ctx_i = ctx.n_ctx() as i32;
    let max_new = MAX_NEW_TOKENS.min(n_ctx_i - prompt_len - 8).max(0);
    if max_new == 0 {
        let msg = "prompt fills the context window".to_string();
        let _ = deltas.send(Err(msg.clone()));
        return Err(msg);
    }
    if to_add.is_empty() {
        let msg = "no new tokens to decode".to_string();
        let _ = deltas.send(Err(msg.clone()));
        return Err(msg);
    }

    let mut batch = LlamaBatch::new(to_add.len().max(512), 1);
    let last_idx = to_add.len() - 1;
    for (i, t) in to_add.iter().enumerate() {
        let pos = common as i32 + i as i32;
        batch
            .add(*t, pos, &[0], i == last_idx)
            .map_err(|e| format!("batch.add prompt: {e}"))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| format!("decode prompt: {e}"))?;
    cached.extend_from_slice(to_add);

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.7),
        LlamaSampler::dist(1234),
    ]);

    let mut parse_state = tmpl
        .streaming_state_oaicompat()
        .map_err(|e| format!("streaming_state_oaicompat: {e}"))?;

    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut n_cur = prompt_len;
    let mut produced: i32 = 0;
    let mut emitted_tool_calls = false;
    let mut hit_eos = false;
    let mut cancelled = false;

    while produced < max_new {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            hit_eos = true;
            break;
        }

        let bytes = model
            .token_to_piece_bytes(token, 64, false, None)
            .map_err(|e| format!("token_to_piece_bytes: {e}"))?;
        let mut piece = String::with_capacity(bytes.len() + 4);
        let _ = decoder.decode_to_string(&bytes, &mut piece, false);

        if !piece.is_empty() {
            let parsed = parse_state
                .update(&piece, true)
                .map_err(|e| format!("parse_state.update: {e}"))?;
            for json in parsed {
                for d in oai_delta_to_agent_deltas(&json, &mut emitted_tool_calls) {
                    if deltas.send(Ok(d)).is_err() {
                        // Receiver dropped (loop abandoned). Just bail.
                        return Ok(());
                    }
                }
            }
        }

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("batch.add gen: {e}"))?;
        n_cur += 1;
        produced += 1;
        ctx.decode(&mut batch)
            .map_err(|e| format!("decode gen: {e}"))?;
        cached.push(token);
    }

    // Final flush; tolerate parser errors here since malformed mid-
    // tool-call output (e.g. due to cancellation) is the most likely
    // cause and we have no better recovery.
    if let Ok(final_parsed) = parse_state.update("", false) {
        for json in final_parsed {
            for d in oai_delta_to_agent_deltas(&json, &mut emitted_tool_calls) {
                let _ = deltas.send(Ok(d));
            }
        }
    }

    let stats = StreamStats {
        provider: "local".to_string(),
        prompt_tokens: Some(prompt_len as u32),
        cached_tokens: Some(common as u32),
        gen_tokens: produced as u32,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    let _ = deltas.send(Ok(AgentDelta::Stats(stats)));

    let finish_reason = if cancelled {
        FinishReason::Cancelled
    } else if emitted_tool_calls {
        FinishReason::ToolCalls
    } else if !hit_eos && produced >= max_new {
        FinishReason::Length
    } else {
        FinishReason::Stop
    };
    let _ = deltas.send(Ok(AgentDelta::Done { finish_reason }));
    Ok(())
}

/// Convert one OpenAI-shaped JSON delta string (as emitted by
/// `ChatParseStateOaicompat::update`) into zero or more `AgentDelta`s.
/// A single chunk can carry both a new tool-call header and an
/// argument fragment, so the function may produce multiple deltas.
fn oai_delta_to_agent_deltas(json: &str, emitted_tool_calls: &mut bool) -> Vec<AgentDelta> {
    let v: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();

    if let Some(content) = v.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            out.push(AgentDelta::Content(content.to_string()));
        }
    }

    if let Some(tcs) = v.get("tool_calls").and_then(Value::as_array) {
        for tc in tcs {
            *emitted_tool_calls = true;
            let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
            let id = tc.get("id").and_then(Value::as_str).map(str::to_string);
            let func = tc.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let args = func
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .map(str::to_string);

            if let Some(id) = id {
                out.push(AgentDelta::ToolCallStart {
                    index,
                    id,
                    name: name.unwrap_or_default(),
                });
                if let Some(args) = args {
                    if !args.is_empty() {
                        out.push(AgentDelta::ToolCallArgs { index, fragment: args });
                    }
                }
            } else if let Some(args) = args {
                if !args.is_empty() {
                    out.push(AgentDelta::ToolCallArgs { index, fragment: args });
                }
            }
        }
    }

    out
}
