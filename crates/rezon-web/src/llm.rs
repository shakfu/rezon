// Thin Tauri wrapper around `rezon_core::llm`. Bridges Tauri's
// `AppHandle` event emission and config-dir resolution onto core's
// `ChatSink` + `&Path` interfaces.

use std::path::PathBuf;
use std::sync::Arc;

use rezon_core::llm::{
    self, ChatMsg, ChatOpts, ChatSink, ChatStats, CloudProviderDef, ModelStatus,
};
use rezon_core::search::SearchState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

pub use rezon_core::llm::LlmState;

/// Forwards `ChatSink` events as Tauri events with the same names the
/// frontend has always listened on.
struct TauriChatSink {
    app: AppHandle,
}

impl TauriChatSink {
    fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ChatSink for TauriChatSink {
    fn on_token(&self, delta: &str) {
        let _ = self.app.emit("chat-token", delta);
    }
    fn on_stats(&self, stats: &ChatStats) {
        let _ = self.app.emit("chat-stats", stats);
    }
    fn on_done(&self, full: &str) {
        let _ = self.app.emit("chat-done", full);
    }
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("app_config_dir: {e}"))
}

pub fn persist_last_model(app: &AppHandle, path: &str) {
    match config_dir(app) {
        Ok(dir) => llm::persist_last_model(&dir, path),
        Err(e) => eprintln!("persist last_model: {e}"),
    }
}

pub fn read_last_model(app: &AppHandle) -> Option<String> {
    config_dir(app).ok().and_then(|d| llm::read_last_model(&d))
}

pub async fn do_load(app: &AppHandle, path: String) -> Result<ModelStatus, String> {
    let _ = app.emit("model-loading", &path);
    let state = app.state::<Arc<LlmState>>();
    let status = state.load(path.clone()).await?;
    persist_last_model(app, &path);
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
pub fn model_status(state: State<'_, Arc<LlmState>>) -> Result<ModelStatus, String> {
    Ok(state.status())
}

#[tauri::command]
pub fn cancel_chat(state: State<'_, Arc<LlmState>>) {
    state.cancel();
}

#[tauri::command]
pub async fn chat(
    app: AppHandle,
    state: State<'_, Arc<LlmState>>,
    mut messages: Vec<ChatMsg>,
    opts: ChatOpts,
) -> Result<String, String> {
    // Expand `[[wikilink]]` markers against the active vault (if
    // any). System message + most recent user turn only; everything
    // else passes through so prompt caching stays valid across turns.
    // Resolution happens here at the send boundary so storage
    // (frontend state) keeps the raw markers.
    let vault = app.state::<Arc<SearchState>>().active_vault();
    if let Some(v) = vault.as_deref() {
        expand_send_msgs(v, &mut messages, &app);
    }
    let sink: Arc<dyn ChatSink> = Arc::new(TauriChatSink::new(app));
    llm::chat(state.inner().as_ref(), messages, opts, sink).await
}

/// Apply wikilink expansion to a chat message vec destined for the
/// LLM. Mutates the system message (if any, at index 0) and the most
/// recent user message; everything else passes through. Unresolved
/// markers are emitted as a `chat-warning` event so the frontend can
/// surface them next to the conversation.
fn expand_send_msgs(vault: &str, msgs: &mut [ChatMsg], app: &AppHandle) {
    if let Some(first) = msgs.first_mut() {
        if first.role == "system" {
            first.content = expand_field(vault, &first.content, app);
        }
    }
    for msg in msgs.iter_mut().rev() {
        if msg.role == "user" {
            msg.content = expand_field(vault, &msg.content, app);
            break;
        }
    }
}

fn expand_field(vault: &str, text: &str, app: &AppHandle) -> String {
    let r = rezon_core::wikilink::expand(vault, text);
    if !r.unresolved.is_empty() {
        let _ = app.emit("chat-warning", format!(
            "wikilink unresolved: {}",
            r.unresolved.join(", ")
        ));
    }
    r.text
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProviderInfo {
    pub key: String,
    pub label: String,
    pub env_var: String,
    pub default_model: String,
    pub recommended_models: Vec<String>,
    pub api_key_set: bool,
    pub user_configurable: bool,
}

impl From<&CloudProviderDef> for CloudProviderInfo {
    fn from(p: &CloudProviderDef) -> Self {
        CloudProviderInfo {
            key: p.key.clone(),
            label: p.label.clone(),
            env_var: p.env_var.clone(),
            default_model: p.default_model.clone(),
            recommended_models: p.recommended_models.clone(),
            api_key_set: p.user_configurable
                || std::env::var(&p.env_var)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
            user_configurable: p.user_configurable,
        }
    }
}

#[tauri::command]
pub fn cloud_providers() -> Vec<CloudProviderInfo> {
    llm::cloud_providers_catalog()
        .iter()
        .map(CloudProviderInfo::from)
        .collect()
}
