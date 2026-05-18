// Unified UI event channel + sink implementations. Chat and agent
// paths both stream events into the REPL through a single mpsc so the
// loop can `tokio::select!` against one receiver.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rezon_core::agent::{AgentEvent, ConfirmationGate, ConfirmationOutcome, EventSink, ToolCall};
use rezon_core::llm::{ChatMsg, ChatSink, ChatStats};
use serde_json::Value;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

#[derive(Debug, Clone)]
pub struct StatsLite {
    pub prompt_tokens: Option<u32>,
    pub gen_tokens: u32,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub enum UiEvent {
    Token(String),
    Stats(StatsLite),
    Done,
    Error(String),
    ToolStart {
        name: String,
    },
    ToolEnd {
        ok: bool,
        summary: String,
    },
    /// Tool awaiting user approval. The agent task blocks on `tx`
    /// until the REPL writes `true` / `false`. Dropping `tx` reads
    /// as denial.
    Confirm {
        name: String,
        arguments: String,
        tx: oneshot::Sender<bool>,
    },
    /// Final agent-loop message vector, serialised back to
    /// `ChatMsg`. The REPL replaces the active conversation's
    /// `messages` with this snapshot so the next agent run sees the
    /// real assistant `tool_calls` + tool-role replies rather than
    /// just the pretty pills shown live.
    AgentHistory(Vec<ChatMsg>),
}

pub struct TuiChatSink {
    tx: UnboundedSender<UiEvent>,
}

impl TuiChatSink {
    pub fn new(tx: UnboundedSender<UiEvent>) -> Self {
        Self { tx }
    }
}

impl ChatSink for TuiChatSink {
    fn on_token(&self, delta: &str) {
        let _ = self.tx.send(UiEvent::Token(delta.to_string()));
    }
    fn on_stats(&self, stats: &ChatStats) {
        let _ = self.tx.send(UiEvent::Stats(StatsLite {
            prompt_tokens: stats.prompt_tokens,
            gen_tokens: stats.gen_tokens,
            duration_ms: stats.duration_ms,
        }));
    }
    fn on_done(&self, _full: &str) {
        let _ = self.tx.send(UiEvent::Done);
    }
}

pub struct TuiAgentSink {
    tx: UnboundedSender<UiEvent>,
}

impl TuiAgentSink {
    pub fn new(tx: UnboundedSender<UiEvent>) -> Self {
        Self { tx }
    }
}

impl EventSink for TuiAgentSink {
    fn emit(&self, event: AgentEvent) {
        let ui = match event {
            AgentEvent::Token(s) => UiEvent::Token(s),
            AgentEvent::Thinking(_) => return,
            AgentEvent::ToolStart { name, .. } => UiEvent::ToolStart { name },
            AgentEvent::ToolEnd {
                ok, result, error, ..
            } => UiEvent::ToolEnd {
                ok,
                summary: summarize_tool_result(ok, result.as_ref(), error.as_deref()),
            },
            AgentEvent::ToolConfirm { .. } => return,
            AgentEvent::Stats(s) => UiEvent::Stats(StatsLite {
                prompt_tokens: s.prompt_tokens,
                gen_tokens: s.gen_tokens,
                duration_ms: s.duration_ms,
            }),
            // The agent loop's `Done` is suppressed here — the
            // spawn block in `agent.rs` sends `AgentHistory` then
            // `Done` after `run_agent` returns, so the REPL gets
            // history persisted before its terminator fires.
            AgentEvent::Done { .. } => return,
            AgentEvent::Cancelled => UiEvent::Error("cancelled".to_string()),
            AgentEvent::Error(e) => UiEvent::Error(e),
        };
        let _ = self.tx.send(ui);
    }
}

fn summarize_tool_result(ok: bool, result: Option<&Value>, error: Option<&str>) -> String {
    if !ok {
        return error.unwrap_or("error").to_string();
    }
    match result {
        Some(v) => {
            let s = v.to_string();
            if s.chars().count() > 200 {
                let truncated: String = s.chars().take(200).collect();
                format!("{truncated}…")
            } else {
                s
            }
        }
        None => "ok".to_string(),
    }
}

pub struct TuiConfirmationGate {
    tx: UnboundedSender<UiEvent>,
    cancelled: Arc<AtomicBool>,
}

impl TuiConfirmationGate {
    pub fn new(tx: UnboundedSender<UiEvent>, cancelled: Arc<AtomicBool>) -> Self {
        Self { tx, cancelled }
    }
}

#[async_trait]
impl ConfirmationGate for TuiConfirmationGate {
    async fn ask(&self, call: &ToolCall) -> ConfirmationOutcome {
        if self.cancelled.load(Ordering::Relaxed) {
            return ConfirmationOutcome::Denied;
        }
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(UiEvent::Confirm {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                tx,
            })
            .is_err()
        {
            return ConfirmationOutcome::Denied;
        }
        match rx.await {
            Ok(true) => ConfirmationOutcome::Approved,
            _ => ConfirmationOutcome::Denied,
        }
    }
}
