// Provider abstraction: anything that produces a stream of normalized
// AgentDeltas given a list of messages and tool schemas. Cloud and
// local backends both implement this trait so the loop is written
// once.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

use crate::agent::delta::AgentDelta;
use crate::agent::message::ChatMessage;

#[derive(Debug, Clone)]
pub struct ProviderOpts {
    /// Specific model identifier the provider should use this turn.
    /// Provider-specific format (e.g. "gpt-4o-mini",
    /// "anthropic/claude-sonnet-4", a GGUF path for local).
    pub model: String,

    /// Hard cap on tokens generated this turn. None lets the
    /// provider use its default.
    pub max_tokens: Option<u32>,

    /// Cooperative cancel flag. Polled by the provider's stream
    /// implementation to abort early.
    pub cancel: Arc<AtomicBool>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Open a streaming completion. The stream yields `AgentDelta`
    /// values until a `Done` delta indicates the turn is complete or
    /// the stream errors.
    /// The returned stream is `'static`: implementations must clone or
    /// otherwise own everything they need from the inputs at
    /// request-build time. This lets the caller mutate `messages`
    /// (e.g. append the assistant turn) while the stream is still
    /// alive.
    async fn stream(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        opts: &ProviderOpts,
    ) -> Result<BoxStream<'static, Result<AgentDelta>>>;
}
