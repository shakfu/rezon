// Provider-neutral Tool trait + registry.
//
// Tools are leaf functions the model can call. The same trait is used
// for the cloud and local agent paths. Schemas are emitted in OpenAI
// shape because both backends consume that format directly:
//   - async-openai's `tools` field
//   - llama-cpp-2's `apply_chat_template_with_tools_oaicompat`

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

// Note: `app` is optional so the loop can be exercised from
// non-Tauri contexts (examples, integration tests). Tools that
// genuinely need it should treat `None` as a runtime error.

/// A single tool the model can invoke. Implementations describe their
/// schema and dispatch synchronously w.r.t. the model turn (they may
/// of course do async I/O internally).
#[async_trait]
pub trait Tool: Send + Sync {
    /// Stable identifier surfaced verbatim to the model. Must match
    /// the OpenAI function-name constraints: a-z, A-Z, 0-9, '_', '-',
    /// length <= 64.
    fn name(&self) -> &str;

    /// One-line natural-language description used by the model for
    /// tool selection. Keep it tight and concrete.
    fn description(&self) -> &str;

    /// JSON Schema for the parameters object. Returned as a
    /// `serde_json::Value` so adapters can wrap it into either
    /// OpenAI's function-calling shape or any other backend format.
    fn parameters(&self) -> Value;

    /// Whether this tool requires explicit user confirmation before
    /// dispatch. Default false; override to true for any tool that
    /// has side effects beyond reading information (shell exec,
    /// file write, network mutations).
    fn requires_confirmation(&self) -> bool {
        false
    }

    /// Execute the tool. `args` is the parsed parameters object as
    /// JSON; implementations are responsible for argument validation
    /// and coercion. The returned `Value` is serialized into a `tool`
    /// role message for the next turn's prompt.
    async fn dispatch(&self, args: Value, ctx: &ToolContext) -> Result<Value, ToolError>;
}

/// Ambient state passed to every tool invocation. Tools should poll
/// `ctx.cancel` for long-running operations and abort promptly.
pub struct ToolContext {
    pub cancel: Arc<AtomicBool>,
    pub app: Option<AppHandle>,
    /// Optional working directory the agent run is rooted in.
    pub workdir: Option<PathBuf>,
}

/// Reasons a tool dispatch can fail. The agent loop maps each variant
/// onto a `tool` role message describing the failure so the model can
/// recover (e.g. retry with different arguments).
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Arguments did not match the tool's schema.
    #[error("argument: {0}")]
    Argument(String),

    /// User rejected the confirmation prompt.
    #[error("denied by user")]
    Denied,

    /// Run was cancelled.
    #[error("cancelled")]
    Cancelled,

    /// Tool raised an error during execution.
    #[error("runtime: {0}")]
    Runtime(#[from] anyhow::Error),
}

/// Identifies a single tool invocation within a turn. `id` is provider-
/// assigned (matches the `tool_call_id` used to thread the result back
/// into the conversation). `arguments` is the raw JSON string the
/// model emitted; tools parse it themselves so they can produce good
/// argument-error messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string the model emitted. Tools parse this in their
    /// dispatch implementation; keeping it as a string avoids losing
    /// information when the model emits invalid JSON.
    pub arguments: String,
}

/// Result of dispatching a single tool call. Persisted alongside the
/// assistant turn (per decision #1) so users can expand the pill in
/// the UI to see the full result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    /// JSON value or { "error": "..." } envelope on failure.
    pub content: Value,
    pub ok: bool,
}

/// Set of tools available for an agent run. Per decision #2 there is
/// no per-conversation tool gate; the registry is constructed once
/// and reused for every run.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    pub fn tools(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.values()
    }

    /// Drop tools whose names appear in `exclude`. Used by the Tauri
    /// command path to honor the user's per-tool "disable" setting
    /// before handing the registry to the loop.
    pub fn without(mut self, exclude: &[String]) -> Self {
        for name in exclude {
            self.tools.remove(name);
        }
        self
    }

    /// OpenAI-shaped tools array, ready to feed both `async-openai`'s
    /// `tools` request field and llama-cpp-2's
    /// `apply_chat_template_with_tools_oaicompat` `tools_json` arg.
    pub fn openai_schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters(),
                    }
                })
            })
            .collect()
    }
}
