// ReAct-style tool-calling agent prototype against an OpenAI-compatible
// chat-completions endpoint via async-openai.
//
// Run:
//   OPENAI_API_KEY=sk-...  cargo run --example react_agent -- "what is 17 * 23, then add the current hour?"
//
// Optional env:
//   OPENAI_BASE_URL  default https://api.openai.com/v1
//   OPENAI_MODEL     default gpt-4o-mini
//   AGENT_MAX_STEPS  default 8
//
// The loop:
//   1. Send messages + tool schemas.
//   2. If the assistant returns tool_calls, execute each tool, append a
//      tool message per call, loop.
//   3. If the assistant returns content with no tool_calls, that is the
//      final answer - print it and stop.
//   4. Hard cap on iterations so a confused model cannot spin forever.

use std::env;

use anyhow::{anyhow, Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionTool, ChatCompletionTools, CreateChatCompletionRequestArgs, FunctionObject,
    },
    Client,
};
use serde::Deserialize;
use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MAX_STEPS: usize = 8;

const SYSTEM_PROMPT: &str = "You are a careful problem-solver. \
Use the provided tools when a calculation, time lookup, or other tool \
capability is needed. Think step by step. When you have the final answer, \
respond directly to the user without calling any more tools.";

fn main() -> Result<()> {
    let user_prompt = env::args()
        .nth(1)
        .unwrap_or_else(|| "What is 17 * 23, then add the current hour to it?".to_string());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(user_prompt))
}

async fn run(user_prompt: String) -> Result<()> {
    let api_key = env::var("OPENAI_API_KEY").context("OPENAI_API_KEY not set")?;
    let base_url = env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let max_steps = env::var("AGENT_MAX_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_STEPS);

    let client = Client::with_config(
        OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url),
    );

    let tools = tool_schemas();

    let mut messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(SYSTEM_PROMPT)
            .build()?
            .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(user_prompt.clone())
            .build()?
            .into(),
    ];

    println!("USER: {user_prompt}\n");

    for step in 1..=max_steps {
        println!("--- step {step} ---");

        let req = CreateChatCompletionRequestArgs::default()
            .model(&model)
            .messages(messages.clone())
            .tools(tools.clone())
            .build()?;

        let resp = client.chat().create(req).await?;
        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("empty choices"))?;
        let assistant = choice.message;

        // Record the assistant turn verbatim so subsequent tool messages
        // can reference its tool_call ids.
        let assistant_req: ChatCompletionRequestAssistantMessage =
            ChatCompletionRequestAssistantMessageArgs::default()
                .content(assistant.content.clone().unwrap_or_default())
                .tool_calls(assistant.tool_calls.clone().unwrap_or_default())
                .build()?;
        messages.push(assistant_req.into());

        if let Some(text) = &assistant.content {
            if !text.is_empty() {
                println!("THOUGHT/REPLY: {text}");
            }
        }

        let tool_calls = assistant.tool_calls.unwrap_or_default();
        if tool_calls.is_empty() {
            println!("\nFINAL: {}", assistant.content.unwrap_or_default());
            return Ok(());
        }

        for wrapped in tool_calls {
            // Only function tool calls are dispatched here; ignore custom-tool
            // variants for this prototype.
            let call = match wrapped {
                ChatCompletionMessageToolCalls::Function(c) => c,
                ChatCompletionMessageToolCalls::Custom(_) => continue,
            };
            let result = dispatch_tool(&call).unwrap_or_else(|e| json!({ "error": e.to_string() }));
            println!(
                "TOOL CALL: {}({}) -> {}",
                call.function.name, call.function.arguments, result
            );
            let tool_msg = ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(call.id.clone())
                .content(result.to_string())
                .build()?;
            messages.push(tool_msg.into());
        }
        println!();
    }

    Err(anyhow!(
        "agent exceeded max_steps={max_steps} without a final answer"
    ))
}

// --- tool schemas exposed to the model -----------------------------------

fn tool_schemas() -> Vec<ChatCompletionTools> {
    vec![
        ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObject {
                name: "calculator".to_string(),
                description: Some(
                    "Evaluate a simple arithmetic expression of the form `a OP b` \
                     where OP is one of + - * / and a, b are numbers."
                        .to_string(),
                ),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "Expression like \"17 * 23\" or \"3.5 + 2\""
                        }
                    },
                    "required": ["expression"]
                })),
                strict: None,
            },
        }),
        ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObject {
                name: "current_time".to_string(),
                description: Some(
                    "Return the current local time as { hour: 0-23, minute: 0-59 }.".to_string(),
                ),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })),
                strict: None,
            },
        }),
    ]
}

// --- tool dispatch -------------------------------------------------------

fn dispatch_tool(call: &ChatCompletionMessageToolCall) -> Result<Value> {
    match call.function.name.as_str() {
        "calculator" => {
            #[derive(Deserialize)]
            struct Args {
                expression: String,
            }
            let args: Args = serde_json::from_str(&call.function.arguments)
                .context("parsing calculator args")?;
            let value = eval_binary(&args.expression)?;
            Ok(json!({ "result": value }))
        }
        "current_time" => {
            // Shell out to `date` so we get the system's local timezone
            // without pulling in chrono/time as a dep for this prototype.
            let out = std::process::Command::new("date")
                .arg("+%H %M %Z")
                .output()
                .context("invoking `date`")?;
            let s = String::from_utf8_lossy(&out.stdout);
            let mut parts = s.split_whitespace();
            let hour: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            let minute: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            let tz = parts.next().unwrap_or("local").to_string();
            Ok(json!({ "hour": hour, "minute": minute, "timezone": tz }))
        }
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

fn eval_binary(expr: &str) -> Result<f64> {
    let s = expr.trim();
    for op in ['+', '-', '*', '/'] {
        if let Some(idx) = find_op(s, op) {
            let (lhs, rhs) = s.split_at(idx);
            let rhs = &rhs[1..];
            let a: f64 = lhs.trim().parse().with_context(|| format!("lhs of {op}"))?;
            let b: f64 = rhs.trim().parse().with_context(|| format!("rhs of {op}"))?;
            return Ok(match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                _ => unreachable!(),
            });
        }
    }
    // Bare number is fine too.
    s.parse::<f64>()
        .with_context(|| format!("could not parse expression `{s}`"))
}

// Find an operator that is not the leading sign of a negative number.
fn find_op(s: &str, op: char) -> Option<usize> {
    s.char_indices()
        .skip(1)
        .find(|(_, c)| *c == op)
        .map(|(i, _)| i)
}
