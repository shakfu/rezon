// Local provider: routes agent runs through the existing
// llama-cpp-2 worker thread via `LlmState::agent_chat_stream`. The
// worker emits AgentDelta values directly; this adapter only has to
// (a) build the OpenAI-shape messages_json + tools_json, (b) submit
// the request, and (c) wrap the receiving channel as a Stream.
//
// Validated path: docs/dev/local_tool_calling.md.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::agent::delta::AgentDelta;
use crate::agent::message::ChatMessage;
use crate::agent::provider::{Provider, ProviderOpts};
use crate::agent::tool::ToolCall;
use crate::llm::LlmState;

pub struct LocalProvider {
    app: AppHandle,
}

impl LocalProvider {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl Provider for LocalProvider {
    async fn stream(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        opts: &ProviderOpts,
    ) -> Result<BoxStream<'static, Result<AgentDelta>>> {
        let messages_json = serde_json::to_string(&messages_to_openai_json(messages))
            .context("serialize messages")?;
        let tools_json =
            serde_json::to_string(&tools).context("serialize tools")?;

        let state = self.app.state::<LlmState>();
        let rx = state
            .agent_chat_stream(messages_json, tools_json, opts.cancel.clone())
            .map_err(|e| anyhow!(e))?;

        // Bridge the worker's tokio mpsc receiver into a futures::Stream
        // by threading the receiver through unfold's state.
        let s = stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Some(Ok(d)) => Some((Ok(d), rx)),
                Some(Err(e)) => Some((Err(anyhow!(e)), rx)),
                None => None,
            }
        });
        Ok(s.boxed())
    }
}

/// Convert internal `ChatMessage` values into OpenAI-shape JSON the
/// llama.cpp template engine expects:
///   - role: "system" | "user" | "assistant" | "tool"
///   - assistant turns may include `tool_calls`
///   - tool turns include `tool_call_id`
///
/// Snake_case fields throughout — that's the wire format the template
/// engine reads, regardless of what rezon's internal serialization uses
/// when talking to the JS layer.
fn messages_to_openai_json(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| match m {
            ChatMessage::System { content } => json!({ "role": "system", "content": content }),
            ChatMessage::User { content } => json!({ "role": "user", "content": content }),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                if tool_calls.is_empty() {
                    json!({ "role": "assistant", "content": content })
                } else {
                    let tcs: Vec<Value> = tool_calls.iter().map(tool_call_to_oai).collect();
                    json!({
                        "role": "assistant",
                        "content": content,
                        "tool_calls": tcs,
                    })
                }
            }
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content,
            }),
        })
        .collect()
}

fn tool_call_to_oai(tc: &ToolCall) -> Value {
    json!({
        "id": tc.id,
        "type": "function",
        "function": {
            "name": tc.name,
            "arguments": tc.arguments,
        }
    })
}
