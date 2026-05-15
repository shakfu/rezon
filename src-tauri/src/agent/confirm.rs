// Confirmation gate: gates each tool dispatch on user approval.
//
// The agent loop calls `gate.ask(call)` before dispatching every tool.
// The gate decides whether to prompt the user, auto-approve, or
// auto-deny based on its own policy table. Two implementations:
//
//   - `AutoApproveGate`: default; always returns Approved. Used by
//     examples and by the production path for tools whose permission
//     is "always".
//   - `TauriConfirmationGate` (in `tauri_gate.rs`): emits an event to
//     the frontend and awaits a oneshot resolved by the
//     `confirm_tool_call` command.

use async_trait::async_trait;

use crate::agent::tool::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationOutcome {
    Approved,
    Denied,
}

#[async_trait]
pub trait ConfirmationGate: Send + Sync {
    async fn ask(&self, call: &ToolCall) -> ConfirmationOutcome;
}

/// Always approves. Suitable for examples, tests, and anywhere the
/// user-confirmation UX does not exist.
pub struct AutoApproveGate;

#[async_trait]
impl ConfirmationGate for AutoApproveGate {
    async fn ask(&self, _call: &ToolCall) -> ConfirmationOutcome {
        ConfirmationOutcome::Approved
    }
}

/// Generates a stable-enough confirmation_id within a single rezon
/// session: timestamp millis + a process-local counter.
pub fn next_confirmation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("conf-{now}-{c}")
}
