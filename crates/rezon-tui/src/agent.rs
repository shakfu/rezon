// Agent session orchestration: builds a provider + tool registry +
// event sink + confirmation gate, then runs `rezon_core::agent::
// run_agent`. Lives in its own module so `app.rs` stays focused on UI
// state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rezon_core::agent::{
    cloud::CloudProvider, local::LocalProvider, run_agent,
    tools::{register_core_tools, register_search_notes},
    AgentOpts, ChatMessage, ConfirmationGate, Provider, ProviderOpts, ToolRegistry,
};
use rezon_core::llm::{resolve_cloud_config, ChatMsg, ChatOpts, LlmState};
use tokio::sync::mpsc::UnboundedSender;

use crate::sink::{TuiAgentSink, TuiConfirmationGate, UiEvent};
use crate::vault::VaultCtx;

/// Run a single agent turn. Spawns the agent loop on a tokio task;
/// returns a handle whose `cancel` flag the App flips on Esc/Ctrl-C.
pub struct AgentRunHandle {
    pub cancel: Arc<AtomicBool>,
}

pub fn spawn_agent_run(
    state: Arc<LlmState>,
    chat_opts: ChatOpts,
    history: Vec<ChatMsg>,
    user_input: String,
    tx: UnboundedSender<UiEvent>,
    max_steps: usize,
    vault: Option<&VaultCtx>,
) -> Result<AgentRunHandle> {
    let messages = build_agent_messages(history, user_input);
    let (provider, model) = build_provider(&state, &chat_opts)?;
    let cancel = Arc::new(AtomicBool::new(false));

    let mut reg = ToolRegistry::new();
    register_core_tools(&mut reg);
    // The model only sees `search_notes` when a vault context exists;
    // the tool itself rejects calls when no vault is open with a
    // human-readable error, so the model can recover.
    if let Some(v) = vault {
        register_search_notes(&mut reg, v.search.clone(), v.embed.clone());
    }
    let registry = Arc::new(reg);

    let sink: Arc<dyn rezon_core::agent::EventSink> = Arc::new(TuiAgentSink::new(tx.clone()));
    let gate: Arc<dyn ConfirmationGate> =
        Arc::new(TuiConfirmationGate::new(tx.clone(), cancel.clone()));

    let opts = AgentOpts {
        provider_opts: ProviderOpts {
            model,
            max_tokens: None,
            cancel: cancel.clone(),
        },
        max_steps,
        gate,
    };

    let tx_err = tx.clone();
    tokio::spawn(async move {
        let mut messages = messages;
        let result = run_agent(provider, registry, sink, &mut messages, opts).await;
        if let Err(e) = result {
            let _ = tx_err.send(UiEvent::Error(e.to_string()));
        }
    });

    Ok(AgentRunHandle { cancel })
}

fn build_provider(
    state: &Arc<LlmState>,
    chat_opts: &ChatOpts,
) -> Result<(Arc<dyn Provider>, String)> {
    if chat_opts.provider == "local" {
        let label = chat_opts
            .model
            .clone()
            .unwrap_or_else(|| "local".to_string());
        let provider: Arc<dyn Provider> = Arc::new(LocalProvider::new(state.clone()));
        return Ok((provider, label));
    }
    let (api_key, base_url, model) =
        resolve_cloud_config(chat_opts).map_err(|e| anyhow!("resolve cloud config: {e}"))?;
    let provider: Arc<dyn Provider> = Arc::new(CloudProvider::new(
        api_key,
        base_url,
        chat_opts.provider.clone(),
    ));
    Ok((provider, model))
}

/// Convert the conversation's flat `ChatMsg` history into the
/// `ChatMessage` form the agent loop wants. Tool-role messages from
/// previous turns are *dropped* because we never recorded their
/// `tool_call_id`; the model treats this as a fresh conversation with
/// only text turns visible. Acceptable for a chat-style UI where tool
/// pills are ephemeral UI artifacts, not part of the model-facing
/// transcript.
fn build_agent_messages(history: Vec<ChatMsg>, user_input: String) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = history
        .into_iter()
        .filter_map(|m| match m.role.as_str() {
            "system" => Some(ChatMessage::System { content: m.content }),
            "user" => Some(ChatMessage::User { content: m.content }),
            "assistant" => Some(ChatMessage::Assistant {
                content: m.content,
                tool_calls: Vec::new(),
            }),
            _ => None,
        })
        .collect();
    out.push(ChatMessage::User {
        content: user_input,
    });
    out
}

impl AgentRunHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
