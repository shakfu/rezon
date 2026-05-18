// Agent module shim: pure types/loop/cloud-provider live in
// `rezon_core::agent`. This file re-exports them so existing
// `crate::agent::...` paths keep resolving, and declares the
// Tauri-specific pieces that still live in rezon-web (commands,
// LocalProvider, TauriEventSink, TauriConfirmationGate, tools/).
//
// Subsequent phases (P3, P4) will move local + tools into core and
// shrink this list further.

pub mod commands;
pub mod local;
pub mod tauri_gate;
pub mod tauri_sink;
pub mod tools;

pub use rezon_core::agent::{
    cloud, confirm, delta, event, loop_, message, provider, tool, AgentDelta, AgentEvent,
    AgentOpts, AgentOutcome, ChatMessage, EventSink, FinishReason, LogEventSink, Provider,
    ProviderOpts, StreamStats, Tool, ToolCall, ToolContext, ToolError, ToolRegistry, ToolResult,
    run_agent,
};
