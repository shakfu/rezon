// Tauri commands: agent_chat and cancel_agent.
//
// Phase 3 supports cloud providers only. Local-model tool calling
// arrives in phase 4 (extends the existing llm worker thread).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::sync::oneshot;

use crate::agent::{
    cloud::CloudProvider, confirm::ConfirmationGate, local::LocalProvider, loop_::AgentOutcome,
    run_agent, tauri_gate::TauriConfirmationGate, tauri_sink::TauriEventSink,
    tools::default_registry, AgentOpts, ChatMessage, Provider, ProviderOpts,
};
use crate::llm;

/// Tracks the cancel flag for the in-flight agent run, if any, plus
/// the table of pending tool-confirmation oneshots. One active run at
/// a time, mirroring `LlmState`'s pattern; starting a new run cancels
/// any previous one.
#[derive(Default)]
pub struct AgentState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// Map of confirmation_id -> oneshot sender. The gate inserts on
    /// prompt; `confirm_tool_call` (or shutdown) removes and resolves.
    pending_confirms: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl AgentState {
    pub fn shutdown(&self) {
        if let Ok(g) = self.cancel.lock() {
            if let Some(c) = g.as_ref() {
                c.store(true, Ordering::Relaxed);
            }
        }
        // Drop any pending confirms; the receiver side will see the
        // sender closed and treat it as denial.
        if let Ok(mut g) = self.pending_confirms.lock() {
            g.clear();
        }
    }

    /// Allocate a oneshot for a pending confirmation. Returns the
    /// receiver; the gate awaits this. The sender is stored under
    /// `id` until `confirm_tool_call` resolves it (or the run is
    /// cancelled).
    pub fn register_pending_confirm(&self, id: String) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut g) = self.pending_confirms.lock() {
            g.insert(id, tx);
        }
        rx
    }

    /// Drop a pending confirmation entry without resolving it (e.g.
    /// the gate's await observed an error). The receiver has already
    /// been consumed; this is just cleanup of the map.
    pub fn cancel_pending_confirm(&self, id: &str) {
        if let Ok(mut g) = self.pending_confirms.lock() {
            g.remove(id);
        }
    }

    fn take_pending_confirm(&self, id: &str) -> Option<oneshot::Sender<bool>> {
        self.pending_confirms
            .lock()
            .ok()
            .and_then(|mut g| g.remove(id))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatOpts {
    /// Cloud provider key: "openai" | "anthropic" | "openrouter" | "other".
    pub provider: String,
    pub model: Option<String>,
    /// Required when `provider == "other"`.
    pub base_url: Option<String>,
    /// Optional override; named providers normally read their key from env.
    pub api_key: Option<String>,
    /// Hard cap on agent loop iterations. Defaults to 8.
    pub max_steps: Option<usize>,
    pub max_tokens: Option<u32>,
    /// Per-tool permissions resolved on the frontend:
    /// "ask" | "always" | "disable". Tools mapped to "disable" are
    /// filtered out of the registry. The remaining map drives the
    /// confirmation gate: "always" auto-approves, "ask" prompts the
    /// user. Missing entries default to "ask".
    #[serde(default)]
    pub tool_permissions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub requires_confirmation: bool,
}

/// Snapshot of registered tools. Used by the Settings UI to render
/// the per-tool permission list.
#[tauri::command]
pub fn tools_catalog() -> Vec<ToolInfo> {
    default_registry()
        .tools()
        .map(|t| ToolInfo {
            name: t.name().to_string(),
            description: t.description().to_string(),
            requires_confirmation: t.requires_confirmation(),
        })
        .collect()
}

#[tauri::command]
pub async fn agent_chat(
    app: AppHandle,
    state: State<'_, AgentState>,
    messages: Vec<ChatMessage>,
    opts: AgentChatOpts,
) -> Result<String, String> {
    let (provider, model): (Arc<dyn Provider>, String) = if opts.provider == "local" {
        // The local model's GGUF path is what `model` carries today
        // for the existing chat command; for the agent path the model
        // identity comes from whatever GGUF is loaded, so we surface
        // a synthetic label for stats rather than rejecting a missing
        // model field.
        let label = opts.model.clone().unwrap_or_else(|| "local".to_string());
        (Arc::new(LocalProvider::new(app.clone())), label)
    } else {
        let (api_key, base_url, model) = resolve_cloud_config(&opts)?;
        let label = opts.provider.clone();
        (
            Arc::new(CloudProvider::new(api_key, base_url, label)),
            model,
        )
    };
    // "disable" tools are stripped from the registry so the model
    // never sees them; the rest are passed to the gate, which decides
    // per-call whether to prompt or auto-approve.
    let disabled: Vec<String> = opts
        .tool_permissions
        .iter()
        .filter(|(_, v)| v.as_str() == "disable")
        .map(|(k, _)| k.clone())
        .collect();
    let registry = Arc::new(default_registry().without(&disabled));
    let sink = Arc::new(TauriEventSink::new(app.clone()));

    // Replace any existing cancel slot with a fresh flag for this run.
    // If a prior run was still active, signal it to abort.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut g = state.cancel.lock().unwrap();
        if let Some(prev) = g.replace(cancel.clone()) {
            prev.store(true, Ordering::Relaxed);
        }
    }

    let gate: Arc<dyn ConfirmationGate> = Arc::new(TauriConfirmationGate::new(
        app.clone(),
        opts.tool_permissions.clone(),
    ));

    let agent_opts = AgentOpts {
        provider_opts: ProviderOpts {
            model,
            max_tokens: opts.max_tokens,
            cancel: cancel.clone(),
        },
        max_steps: opts.max_steps.unwrap_or(8),
        gate,
    };

    let mut messages = messages;
    let result = run_agent(provider, registry, sink, &mut messages, agent_opts).await;

    // Clear the active cancel slot only if it is still ours; concurrent
    // re-entry may have already replaced it.
    {
        let mut g = state.cancel.lock().unwrap();
        if let Some(active) = g.as_ref() {
            if Arc::ptr_eq(active, &cancel) {
                *g = None;
            }
        }
    }

    match result {
        Ok(AgentOutcome::Final(s)) => Ok(s),
        Ok(AgentOutcome::Cancelled) => Err("cancelled".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn cancel_agent(state: State<'_, AgentState>) {
    let g = state.cancel.lock().unwrap();
    if let Some(c) = g.as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

/// Frontend's reply to an `agent-tool-confirm` event. Resolves the
/// pending oneshot held by `AgentState`; the gate's `await` then
/// proceeds with Approved or Denied. No-op if `confirmation_id` is
/// unknown (e.g. the run was cancelled before the user replied).
#[tauri::command]
pub fn confirm_tool_call(
    state: State<'_, AgentState>,
    confirmation_id: String,
    approved: bool,
) {
    if let Some(tx) = state.take_pending_confirm(&confirmation_id) {
        let _ = tx.send(approved);
    }
}

/// Resolve api_key + base_url + model from the cloud provider catalog
/// in `llm.rs`. Mirrors the resolution `llm::chat` does for non-tool
/// chats so behavior stays consistent.
fn resolve_cloud_config(opts: &AgentChatOpts) -> Result<(String, String, String), String> {
    let def = llm::cloud_provider_def(&opts.provider)
        .ok_or_else(|| format!("unknown provider: {}", opts.provider))?;

    let (api_key, base_url) = if def.user_configurable {
        let base_url = opts
            .base_url
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "base URL is required".to_string())?
            .to_string();
        let api_key = opts
            .api_key
            .clone()
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
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            if def.default_model.is_empty() {
                None
            } else {
                Some(def.default_model.to_string())
            }
        })
        .ok_or_else(|| "model is required".to_string())?;

    Ok((api_key, base_url, model))
}
