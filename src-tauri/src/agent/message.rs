// Internal chat-message representation passed between the agent loop
// and provider adapters. Modeled as an enum so impossible states
// (e.g. tool_calls on a user message) are unrepresentable.

use serde::{Deserialize, Serialize};

use crate::agent::tool::ToolCall;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
// Variants stay snake_case ("system", "user", "assistant", "tool") to
// match OpenAI's role naming. Fields (`tool_calls`, `tool_call_id`)
// switch to camelCase via the inner attribute below so they line up
// with the rest of the rezo JS types (e.g. `MsgStats`).
#[serde(rename_all_fields = "camelCase")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        content: String,
        /// Tool calls the assistant turn requested. Empty when the
        /// turn was a final answer.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// Result of dispatching a tool call. Threaded back to the model
    /// as the next turn's input.
    Tool {
        tool_call_id: String,
        /// Stringified JSON. Tool results may be objects, scalars, or
        /// error envelopes; we serialize at the tool-dispatch site
        /// and just thread the string here.
        content: String,
    },
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: content.into(),
        }
    }
}
