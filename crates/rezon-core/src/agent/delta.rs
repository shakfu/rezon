// The normalized stream type produced by every Provider adapter and
// consumed by the agent loop. Both the cloud (async-openai) and local
// (llama-cpp-2 ChatParseStateOaicompat) adapters map their native
// chunk shapes onto this enum so the loop is written once.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentDelta {
    /// Visible text fragment for the assistant turn.
    Content(String),

    /// Reasoning/thinking block fragment. Qwen 3's <think>...</think>,
    /// or a future Anthropic native-thinking block. Per design
    /// decision #4 these stream to the UI as a "thinking..." indicator
    /// only; full content is buffered for on-demand expansion.
    Thinking(String),

    /// First chunk of a tool call. Carries the index, tool_call_id,
    /// and tool name. `arguments` may be empty here and arrive in
    /// subsequent ToolCallArgs deltas.
    ToolCallStart {
        index: u32,
        id: String,
        name: String,
    },

    /// Argument fragment for an in-progress tool call, identified by
    /// `index`. Concatenate fragments for the same index to assemble
    /// the final JSON arguments string.
    ToolCallArgs { index: u32, fragment: String },

    /// Optional explicit end-of-tool-call marker. Cloud streams
    /// usually omit this and signal completion via Done; local
    /// streams may emit it. Loop tolerates either.
    ToolCallEnd { index: u32 },

    /// Provider-specific stats (token counts, cache usage). Emitted
    /// once per turn near the end of the stream.
    Stats(StreamStats),

    /// End of this assistant turn. The loop inspects `finish_reason`
    /// to decide whether to dispatch tools and continue, or return
    /// the final answer.
    Done { finish_reason: FinishReason },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Model returned a final answer; no further turns.
    Stop,
    /// Model requested tool calls; loop should dispatch and continue.
    ToolCalls,
    /// Hit the model or context token limit.
    Length,
    /// User cancelled mid-stream.
    Cancelled,
    /// Anything provider-specific not covered above.
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStats {
    /// Provider key (e.g. "openai", "anthropic", "openrouter", "local").
    pub provider: String,
    pub prompt_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub gen_tokens: u32,
    pub duration_ms: u64,
}
