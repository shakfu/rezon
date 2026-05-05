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
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

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

struct CloudProviderDef {
    key: &'static str,
    label: &'static str,
    env_var: &'static str,
    base_url: &'static str,
    default_model: &'static str,
    recommended: &'static [&'static str],
    user_configurable: bool,
}

static OPENAI_PROVIDER: CloudProviderDef = CloudProviderDef {
    key: "openai",
    label: "OpenAI",
    env_var: "OPENAI_API_KEY",
    base_url: "https://api.openai.com/v1",
    default_model: "gpt-5.4-mini",
    recommended: &[
        "gpt-5.4-nano",
        "gpt-5.4-mini",
        "gpt-5.4",
        "gpt-5.4-pro",
        "gpt-5.5",
        "gpt-5.5-pro",
    ],
    user_configurable: false,
};

static ANTHROPIC_PROVIDER: CloudProviderDef = CloudProviderDef {
    key: "anthropic",
    label: "Anthropic",
    env_var: "ANTHROPIC_API_KEY",
    base_url: "https://api.anthropic.com/v1",
    default_model: "claude-sonnet-4-6",
    recommended: &[
        "claude-opus-4-7",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
    ],
    user_configurable: false,
};

static OPENROUTER_PROVIDER: CloudProviderDef = CloudProviderDef {
    key: "openrouter",
    label: "OpenRouter",
    env_var: "OPENROUTER_API_KEY",
    base_url: "https://openrouter.ai/api/v1",
    default_model: "deepseek/deepseek-v3.2",
    recommended: &[
        "deepseek/deepseek-v3.2",
        "deepseek/deepseek-v4-flash",
        "deepseek/deepseek-v4-pro",
        "google/gemini-2.5-flash",
        "google/gemini-2.5-flash-lite",
        "google/gemini-3-flash-preview",
        "google/gemma-4-26b-a4b-it:free",
        "google/gemma-4-31b-it:free",
        "moonshotai/kimi-k2.5",
        "moonshotai/kimi-k2.6",
        "nvidia/nemotron-3-super-120b-a12b:free",
        "openrouter/owl-alpha",
        "qwen/qwen3-235b-a22b-2507",
        "qwen/qwen3.5-flash-02-23",
        "qwen/qwen3.6-plus",
        "x-ai/grok-4.1-fast",
        "x-ai/grok-4.3",
    ],
    user_configurable: false,
};

static OTHER_PROVIDER: CloudProviderDef = CloudProviderDef {
    key: "other",
    label: "Other",
    env_var: "",
    base_url: "",
    default_model: "",
    recommended: &[],
    user_configurable: true,
};

static CLOUD_PROVIDERS: &[&CloudProviderDef] = &[
    &OPENAI_PROVIDER,
    &ANTHROPIC_PROVIDER,
    &OPENROUTER_PROVIDER,
    &OTHER_PROVIDER,
];

fn cloud_provider_def(key: &str) -> Option<&'static CloudProviderDef> {
    CLOUD_PROVIDERS.iter().copied().find(|p| p.key == key)
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
    CLOUD_PROVIDERS
        .iter()
        .map(|p| CloudProviderInfo {
            key: p.key.to_string(),
            label: p.label.to_string(),
            env_var: p.env_var.to_string(),
            default_model: p.default_model.to_string(),
            recommended_models: p.recommended.iter().map(|s| s.to_string()).collect(),
            // Treat user-configurable providers as always available; the
            // user supplies the key (if any) at request time.
            api_key_set: p.user_configurable
                || std::env::var(p.env_var)
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
        let api_key = std::env::var(def.env_var)
            .ok()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{} is not set", def.env_var))?;
        (api_key, def.base_url.to_string())
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

    while let Ok(WorkerRequest::Chat {
        messages,
        cancel,
        app,
        respond,
    }) = rx.recv()
    {
        let result = run_chat_with_cache(&app, model_ref, &mut ctx, &mut cached, messages, cancel);
        let _ = respond.send(result);
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
