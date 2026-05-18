// Provider-agnostic agent loop + supporting types.
//
// The Tauri-specific pieces (TauriEventSink, TauriConfirmationGate, the
// LocalProvider that reaches into LlmState, and the actual tool
// implementations) live in their respective shell crates (rezon-web
// today; rezon-tui in a later phase).

pub mod cloud;
pub mod confirm;
pub mod delta;
pub mod event;
pub mod loop_;
pub mod message;
pub mod provider;
pub mod tool;

pub use cloud::CloudProvider;
pub use confirm::{AutoApproveGate, ConfirmationGate, ConfirmationOutcome, next_confirmation_id};
pub use delta::{AgentDelta, FinishReason, StreamStats};
pub use event::{AgentEvent, EventSink, LogEventSink};
pub use loop_::{run_agent, AgentOpts, AgentOutcome};
pub use message::ChatMessage;
pub use provider::{Provider, ProviderOpts};
pub use tool::{Tool, ToolCall, ToolContext, ToolError, ToolRegistry, ToolResult};
