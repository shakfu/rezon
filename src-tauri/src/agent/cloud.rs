// Cloud provider: wraps async-openai's chat-completions streaming
// endpoint. Works against any OpenAI-compatible base URL (OpenAI
// proper, Anthropic's compat endpoint, OpenRouter, Ollama, ...).
//
// The mapping from `ChatCompletionStreamResponseDelta` chunks onto
// our normalized `AgentDelta` enum is the bulk of this file.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionStreamOptions, ChatCompletionTool, ChatCompletionTools,
    CreateChatCompletionRequestArgs, FinishReason as OaiFinish, FunctionCall, FunctionObject,
};
use async_openai::Client;
use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use serde_json::Value;

use crate::agent::delta::{AgentDelta, FinishReason, StreamStats};
use crate::agent::message::ChatMessage;
use crate::agent::provider::{Provider, ProviderOpts};
use crate::agent::tool::ToolCall;

/// Identifier surfaced in `StreamStats.provider`. Set per-instance so
/// downstream code can distinguish OpenAI / Anthropic / OpenRouter.
pub struct CloudProvider {
    client: Client<OpenAIConfig>,
    provider_label: String,
}

impl CloudProvider {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>, label: impl Into<String>) -> Self {
        let cfg = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);
        Self {
            client: Client::with_config(cfg),
            provider_label: label.into(),
        }
    }
}

#[async_trait]
impl Provider for CloudProvider {
    async fn stream(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        opts: &ProviderOpts,
    ) -> Result<BoxStream<'static, Result<AgentDelta>>> {
        let oai_msgs = to_openai_messages(messages)?;
        let oai_tools = to_openai_tools(tools)?;

        let mut req = CreateChatCompletionRequestArgs::default();
        req.model(&opts.model)
            .messages(oai_msgs)
            .stream_options(ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            });
        if !oai_tools.is_empty() {
            req.tools(oai_tools);
        }
        if let Some(max) = opts.max_tokens {
            req.max_tokens(max);
        }
        let request = req.build().context("build chat request")?;

        let started = std::time::Instant::now();
        let upstream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .context("create_stream")?;

        let cancel = opts.cancel.clone();
        let provider_label = self.provider_label.clone();

        // Per-chunk state kept across the unfold:
        // - `seen_indexes`: which tool-call indexes have already received
        //   a Start delta. The first chunk for an index carries `id`
        //   and `function.name`; subsequent chunks carry only argument
        //   fragments.
        // - `pending_done`: queued Done delta to emit after Stats.
        //   We keep emission single-yield-per-poll to keep the stream
        //   shape simple.
        let state = ChunkState {
            upstream: upstream.boxed(),
            cancel,
            provider_label,
            started,
            queue: Vec::new(),
            seen_indexes: Vec::new(),
            saw_finish: None,
            done_emitted: false,
        };

        let stream = stream::unfold(state, |mut s| async move {
            loop {
                if let Some(d) = s.queue.pop() {
                    return Some((Ok(d), s));
                }
                if s.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    if !s.done_emitted {
                        s.done_emitted = true;
                        return Some((
                            Ok(AgentDelta::Done {
                                finish_reason: FinishReason::Cancelled,
                            }),
                            s,
                        ));
                    }
                    return None;
                }

                let next = s.upstream.next().await;
                match next {
                    None => {
                        if s.done_emitted {
                            return None;
                        }
                        s.done_emitted = true;
                        let reason = s
                            .saw_finish
                            .take()
                            .map(map_finish_reason)
                            .unwrap_or(FinishReason::Stop);
                        return Some((Ok(AgentDelta::Done { finish_reason: reason }), s));
                    }
                    Some(Err(e)) => {
                        return Some((Err(anyhow!("upstream stream error: {e}")), s));
                    }
                    Some(Ok(resp)) => {
                        // Drain choices into queued deltas. Reverse
                        // because we pop from the back.
                        let mut produced: Vec<AgentDelta> = Vec::new();
                        for choice in resp.choices {
                            if let Some(content) = choice.delta.content {
                                if !content.is_empty() {
                                    produced.push(AgentDelta::Content(content));
                                }
                            }
                            if let Some(tcs) = choice.delta.tool_calls {
                                for chunk in tcs {
                                    let idx = chunk.index;
                                    let is_first = !s.seen_indexes.contains(&idx);
                                    if is_first {
                                        s.seen_indexes.push(idx);
                                        let id = chunk.id.unwrap_or_default();
                                        let name = chunk
                                            .function
                                            .as_ref()
                                            .and_then(|f| f.name.clone())
                                            .unwrap_or_default();
                                        produced.push(AgentDelta::ToolCallStart {
                                            index: idx,
                                            id,
                                            name,
                                        });
                                        if let Some(args) =
                                            chunk.function.as_ref().and_then(|f| f.arguments.clone())
                                        {
                                            if !args.is_empty() {
                                                produced.push(AgentDelta::ToolCallArgs {
                                                    index: idx,
                                                    fragment: args,
                                                });
                                            }
                                        }
                                    } else if let Some(args) =
                                        chunk.function.and_then(|f| f.arguments)
                                    {
                                        if !args.is_empty() {
                                            produced.push(AgentDelta::ToolCallArgs {
                                                index: idx,
                                                fragment: args,
                                            });
                                        }
                                    }
                                }
                            }
                            if let Some(reason) = choice.finish_reason {
                                s.saw_finish = Some(reason);
                            }
                        }

                        if let Some(usage) = resp.usage {
                            let stats = StreamStats {
                                provider: s.provider_label.clone(),
                                prompt_tokens: Some(usage.prompt_tokens),
                                cached_tokens: usage
                                    .prompt_tokens_details
                                    .as_ref()
                                    .and_then(|d| d.cached_tokens),
                                gen_tokens: usage.completion_tokens,
                                duration_ms: s.started.elapsed().as_millis() as u64,
                            };
                            produced.push(AgentDelta::Stats(stats));
                        }

                        // Push in reverse so Vec::pop yields in original order.
                        s.queue.extend(produced.into_iter().rev());
                        if !s.queue.is_empty() {
                            let d = s.queue.pop().unwrap();
                            return Some((Ok(d), s));
                        }
                        // Empty chunk (e.g. just a heartbeat) — loop and pull next.
                    }
                }
            }
        });

        Ok(stream.boxed())
    }
}

struct ChunkState {
    upstream: BoxStream<'static, std::result::Result<async_openai::types::chat::CreateChatCompletionStreamResponse, async_openai::error::OpenAIError>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    provider_label: String,
    started: std::time::Instant,
    /// Buffer in *reverse* emission order; pop from the back.
    queue: Vec<AgentDelta>,
    seen_indexes: Vec<u32>,
    saw_finish: Option<OaiFinish>,
    done_emitted: bool,
}

fn map_finish_reason(r: OaiFinish) -> FinishReason {
    match r {
        OaiFinish::Stop => FinishReason::Stop,
        OaiFinish::Length => FinishReason::Length,
        OaiFinish::ToolCalls => FinishReason::ToolCalls,
        OaiFinish::ContentFilter => FinishReason::Other("content_filter".to_string()),
        OaiFinish::FunctionCall => FinishReason::Other("function_call".to_string()),
    }
}

fn to_openai_tools(tools: &[Value]) -> Result<Vec<ChatCompletionTools>> {
    tools
        .iter()
        .map(|t| {
            let func = t
                .get("function")
                .ok_or_else(|| anyhow!("tool schema missing `function`"))?;
            let name = func
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tool schema missing function.name"))?
                .to_string();
            let description = func
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            let parameters = func.get("parameters").cloned();
            Ok(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name,
                    description,
                    parameters,
                    strict: None,
                },
            }))
        })
        .collect()
}

fn to_openai_messages(messages: &[ChatMessage]) -> Result<Vec<ChatCompletionRequestMessage>> {
    messages
        .iter()
        .map(|m| match m {
            ChatMessage::System { content } => Ok(ChatCompletionRequestSystemMessageArgs::default()
                .content(content.clone())
                .build()
                .context("system message")?
                .into()),
            ChatMessage::User { content } => Ok(ChatCompletionRequestUserMessageArgs::default()
                .content(content.clone())
                .build()
                .context("user message")?
                .into()),
            ChatMessage::Assistant { content, tool_calls } => {
                let oai_calls: Vec<ChatCompletionMessageToolCalls> = tool_calls
                    .iter()
                    .map(|tc: &ToolCall| {
                        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                            id: tc.id.clone(),
                            function: FunctionCall {
                                name: tc.name.clone(),
                                arguments: tc.arguments.clone(),
                            },
                        })
                    })
                    .collect();
                let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
                builder.content(content.clone());
                if !oai_calls.is_empty() {
                    builder.tool_calls(oai_calls);
                }
                let built: ChatCompletionRequestAssistantMessage =
                    builder.build().context("assistant message")?;
                Ok(built.into())
            }
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => Ok(ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id.clone())
                .content(content.clone())
                .build()
                .context("tool message")?
                .into()),
        })
        .collect()
}
