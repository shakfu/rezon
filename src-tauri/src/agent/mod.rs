// Agent module: provider-agnostic, single-agent multi-tool loop.
//
// See docs/dev/agent_loop_design.md for the full design and the seven
// decisions that pin it down.
//
// Phase 1 (this file + tool.rs + delta.rs + tools/) lays down the
// types. The loop, provider adapters, Tauri command surface, and
// confirmation flow are subsequent phases.

pub mod cloud;
pub mod commands;
pub mod confirm;
pub mod delta;
pub mod event;
pub mod local;
pub mod loop_;
pub mod message;
pub mod provider;
pub mod tauri_gate;
pub mod tauri_sink;
pub mod tool;
pub mod tools;

pub use delta::{AgentDelta, FinishReason, StreamStats};
pub use event::{AgentEvent, EventSink, LogEventSink};
pub use loop_::{run_agent, AgentOpts, AgentOutcome};
pub use message::ChatMessage;
pub use provider::{Provider, ProviderOpts};
pub use tool::{Tool, ToolCall, ToolContext, ToolError, ToolRegistry, ToolResult};
