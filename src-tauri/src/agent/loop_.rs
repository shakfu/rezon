// The agent loop. Provider-agnostic: takes a Provider, a ToolRegistry,
// an EventSink, and an initial message vector; runs until either the
// model returns a final answer, the user cancels, or max_steps is hit.
//
// Phase 2 caveats:
//   - Confirmation flow is stubbed: tools that report
//     `requires_confirmation = true` are still dispatched in this
//     phase, with a one-line warning. Phase 5 wires the real confirm
//     UX.
//   - Single conversation, no persistence yet. The caller is
//     responsible for storing the message vector if it cares.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde_json::Value;

use crate::agent::confirm::{ConfirmationGate, ConfirmationOutcome};
use crate::agent::delta::{AgentDelta, FinishReason};
use crate::agent::event::{AgentEvent, EventSink};
use crate::agent::message::ChatMessage;
use crate::agent::provider::{Provider, ProviderOpts};
use crate::agent::tool::{Tool, ToolCall, ToolContext, ToolError, ToolRegistry};

#[derive(Clone)]
pub struct AgentOpts {
    pub provider_opts: ProviderOpts,
    pub max_steps: usize,
    /// Gates each tool dispatch on user approval. The default
    /// `AutoApproveGate` makes the loop behave as before; production
    /// rezon passes a `TauriConfirmationGate` that prompts the user.
    pub gate: Arc<dyn ConfirmationGate>,
}

#[derive(Debug)]
pub enum AgentOutcome {
    Final(String),
    Cancelled,
}

/// Run a single agent session. Mutates `messages` to reflect the
/// assistant turn(s) and tool-result turn(s) produced during the run.
pub async fn run_agent(
    provider: Arc<dyn Provider>,
    registry: Arc<ToolRegistry>,
    sink: Arc<dyn EventSink>,
    messages: &mut Vec<ChatMessage>,
    opts: AgentOpts,
) -> Result<AgentOutcome> {
    let cancel = opts.provider_opts.cancel.clone();

    for _step in 1..=opts.max_steps {
        if cancel.load(Ordering::Relaxed) {
            sink.emit(AgentEvent::Cancelled);
            return Ok(AgentOutcome::Cancelled);
        }

        let tools = registry.openai_schemas();
        let mut stream = provider
            .stream(messages.as_slice(), &tools, &opts.provider_opts)
            .await?;

        let mut acc = TurnAccumulator::default();
        while let Some(item) = stream.next().await {
            match item? {
                AgentDelta::Content(s) => {
                    acc.content.push_str(&s);
                    sink.emit(AgentEvent::Token(s));
                }
                AgentDelta::Thinking(s) => {
                    acc.thinking.push_str(&s);
                    sink.emit(AgentEvent::Thinking(s));
                }
                AgentDelta::ToolCallStart { index, id, name } => {
                    acc.tool_calls.insert(
                        index,
                        ToolCallBuilder {
                            id,
                            name,
                            args: String::new(),
                        },
                    );
                }
                AgentDelta::ToolCallArgs { index, fragment } => {
                    if let Some(b) = acc.tool_calls.get_mut(&index) {
                        b.args.push_str(&fragment);
                    }
                }
                AgentDelta::ToolCallEnd { .. } => {
                    // Index already complete in `acc.tool_calls`; nothing to do.
                }
                AgentDelta::Stats(s) => sink.emit(AgentEvent::Stats(s)),
                AgentDelta::Done { finish_reason } => {
                    acc.finish_reason = finish_reason;
                    break;
                }
            }
        }

        // Build the assistant turn from the accumulated state.
        let assistant_calls: Vec<ToolCall> = acc
            .tool_calls
            .into_values()
            .map(|b| ToolCall {
                id: b.id,
                name: b.name,
                arguments: b.args,
            })
            .collect();
        messages.push(ChatMessage::Assistant {
            content: acc.content.clone(),
            tool_calls: assistant_calls.clone(),
        });

        match acc.finish_reason {
            FinishReason::Cancelled => {
                sink.emit(AgentEvent::Cancelled);
                return Ok(AgentOutcome::Cancelled);
            }
            FinishReason::Stop | FinishReason::Length | FinishReason::Other(_) => {
                sink.emit(AgentEvent::Done {
                    content: acc.content.clone(),
                });
                return Ok(AgentOutcome::Final(acc.content));
            }
            FinishReason::ToolCalls => { /* fall through to dispatch */ }
        }

        if assistant_calls.is_empty() {
            // Provider signaled tool_calls but emitted none. Treat as final
            // to avoid a loop with no progress.
            sink.emit(AgentEvent::Done {
                content: acc.content.clone(),
            });
            return Ok(AgentOutcome::Final(acc.content));
        }

        for call in &assistant_calls {
            if cancel.load(Ordering::Relaxed) {
                sink.emit(AgentEvent::Cancelled);
                return Ok(AgentOutcome::Cancelled);
            }

            // Ask the user (or the gate's policy) for approval
            // before announcing ToolStart. Denied calls still emit a
            // ToolEnd so the UI's pill collapses to an error state,
            // and a tool message is appended to the history so the
            // model can react.
            let outcome = opts.gate.ask(call).await;
            if matches!(outcome, ConfirmationOutcome::Denied) {
                sink.emit(AgentEvent::ToolStart {
                    id: call.id.clone(),
                    name: call.name.clone(),
                });
                sink.emit(AgentEvent::ToolEnd {
                    id: call.id.clone(),
                    ok: false,
                    result: None,
                    error: Some("denied by user".to_string()),
                });
                let content = serde_json::to_string(&serde_json::json!({
                    "error": "denied by user"
                }))
                .unwrap_or_else(|_| "{\"error\":\"denied\"}".to_string());
                messages.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
                continue;
            }

            sink.emit(AgentEvent::ToolStart {
                id: call.id.clone(),
                name: call.name.clone(),
            });

            let result = dispatch_one(&registry, call, &cancel).await;
            match &result {
                Ok(value) => sink.emit(AgentEvent::ToolEnd {
                    id: call.id.clone(),
                    ok: true,
                    result: Some(value.clone()),
                    error: None,
                }),
                Err(e) => sink.emit(AgentEvent::ToolEnd {
                    id: call.id.clone(),
                    ok: false,
                    result: None,
                    error: Some(e.to_string()),
                }),
            }

            // Append a tool message regardless of success so the model
            // can recover from errors on the next turn.
            let content = match &result {
                Ok(v) => v.to_string(),
                Err(e) => serde_json::to_string(&serde_json::json!({
                    "error": e.to_string()
                }))
                .unwrap_or_else(|_| "{\"error\":\"<unserializable>\"}".to_string()),
            };
            messages.push(ChatMessage::Tool {
                tool_call_id: call.id.clone(),
                content,
            });
        }
    }

    let msg = format!("agent exceeded max_steps={}", opts.max_steps);
    sink.emit(AgentEvent::Error(msg.clone()));
    Err(anyhow!(msg))
}

async fn dispatch_one(
    registry: &ToolRegistry,
    call: &ToolCall,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, ToolError> {
    let tool = registry
        .get(&call.name)
        .ok_or_else(|| ToolError::Argument(format!("unknown tool `{}`", call.name)))?
        .clone();

    if tool.requires_confirmation() {
        // Phase-2 stub: we don't have the confirm flow yet. Log and
        // dispatch anyway. Phase 5 will replace this with a blocking
        // confirmation request emitted as an `AgentEvent::ToolConfirm`.
        eprintln!(
            "warn: tool `{}` requires confirmation; phase-2 dispatches without prompting",
            tool.name()
        );
    }

    let args: Value = if call.arguments.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(&call.arguments).map_err(|e| {
            ToolError::Argument(format!("arguments not valid JSON: {e} (raw: {})", call.arguments))
        })?
    };

    let ctx = ToolContext {
        cancel: cancel.clone(),
        app: None,
        workdir: None,
    };
    dispatch_tool(tool.as_ref(), args, &ctx).await
}

async fn dispatch_tool(
    tool: &dyn Tool,
    args: Value,
    ctx: &ToolContext,
) -> Result<Value, ToolError> {
    tool.dispatch(args, ctx).await
}

struct TurnAccumulator {
    content: String,
    thinking: String,
    tool_calls: BTreeMap<u32, ToolCallBuilder>,
    finish_reason: FinishReason,
}

impl Default for TurnAccumulator {
    fn default() -> Self {
        Self {
            content: String::new(),
            thinking: String::new(),
            tool_calls: BTreeMap::new(),
            // Default to Stop; overwritten when a Done delta arrives.
            finish_reason: FinishReason::Stop,
        }
    }
}

struct ToolCallBuilder {
    id: String,
    name: String,
    args: String,
}
