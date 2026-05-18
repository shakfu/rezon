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
// with the rest of the rezon JS types (e.g. `MsgStats`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_user_roundtrip() {
        let m = ChatMessage::system("you are terse");
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"role\":\"system\""));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ChatMessage::System { content } if content == "you are terse"));
    }

    #[test]
    fn assistant_with_tool_calls_serialises_camel_case_field() {
        let m = ChatMessage::Assistant {
            content: "calling".into(),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "current_time".into(),
                arguments: "{}".into(),
            }],
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"role\":\"assistant\""));
        // The enum has `#[serde(rename_all_fields = "camelCase")]`,
        // so the wire field name is `toolCalls`.
        assert!(
            json.contains("\"toolCalls\""),
            "expected camelCase field: {json}"
        );
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        match back {
            ChatMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "current_time");
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn assistant_without_tool_calls_skips_field() {
        let m = ChatMessage::Assistant {
            content: "hi".into(),
            tool_calls: vec![],
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("toolCalls"),
            "empty tool_calls should be skipped: {json}"
        );
    }

    #[test]
    fn tool_message_uses_tool_call_id_camel_case() {
        let m = ChatMessage::Tool {
            tool_call_id: "call-1".into(),
            content: "{\"ok\":true}".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"toolCallId\":\"call-1\""));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ChatMessage::Tool { tool_call_id, .. } if tool_call_id == "call-1"));
    }
}
