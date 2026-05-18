// Tauri implementation of `ConfirmationGate`.
//
// On `ask`, consults the per-tool permission map. "always" returns
// Approved immediately. "ask" allocates a confirmation_id, registers a
// oneshot in `AgentState`, emits an `agent-tool-confirm` event, and
// awaits the user's reply. The `confirm_tool_call` Tauri command (in
// commands.rs) resolves the oneshot.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent::commands::AgentState;
use crate::agent::confirm::{next_confirmation_id, ConfirmationGate, ConfirmationOutcome};
use crate::agent::tool::ToolCall;

pub struct TauriConfirmationGate {
    app: AppHandle,
    /// Resolved per-tool permissions for this run. Tools not present
    /// default to "ask" (defensive).
    permissions: HashMap<String, String>,
}

impl TauriConfirmationGate {
    pub fn new(app: AppHandle, permissions: HashMap<String, String>) -> Self {
        Self { app, permissions }
    }

    fn permission_for(&self, tool: &str) -> &str {
        self.permissions
            .get(tool)
            .map(String::as_str)
            .unwrap_or("ask")
    }
}

#[async_trait]
impl ConfirmationGate for TauriConfirmationGate {
    async fn ask(&self, call: &ToolCall) -> ConfirmationOutcome {
        match self.permission_for(&call.name) {
            "always" => ConfirmationOutcome::Approved,
            // "disable" tools are filtered before the loop; if one
            // somehow gets here, deny rather than auto-approve.
            "disable" => ConfirmationOutcome::Denied,
            // Anything else (incl. "ask", missing, unknown values) -> prompt.
            _ => prompt_user(&self.app, call).await,
        }
    }
}

async fn prompt_user(app: &AppHandle, call: &ToolCall) -> ConfirmationOutcome {
    let id = next_confirmation_id();

    // Register synchronously, drop the State borrow before awaiting.
    let rx = {
        let state = app.state::<AgentState>();
        state.register_pending_confirm(id.clone())
    };

    let _ = app.emit(
        "agent-tool-confirm",
        &json!({
            "confirmationId": id,
            "name": call.name,
            "arguments": call.arguments,
        }),
    );

    match rx.await {
        Ok(approved) => {
            if approved {
                ConfirmationOutcome::Approved
            } else {
                ConfirmationOutcome::Denied
            }
        }
        Err(_) => {
            // Sender dropped (run cancelled / app shutting down).
            let state = app.state::<AgentState>();
            state.cancel_pending_confirm(&id);
            ConfirmationOutcome::Denied
        }
    }
}
