// Phase-2 end-to-end runner. Wires CloudProvider + the default tool
// registry + LogEventSink and runs an agent session against an
// OpenAI-compatible endpoint (default: OpenRouter).
//
// Run:
//   ./examples/run_agent_e2e.sh "<prompt>"
//
// Environment:
//   OPENROUTER_API_KEY  required
//   OPENROUTER_MODEL    optional (default anthropic/claude-sonnet-4)

use std::env;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use rezon_lib::agent::{
    cloud::CloudProvider, confirm::AutoApproveGate, run_agent, tools::default_registry, AgentOpts,
    ChatMessage, LogEventSink, Provider, ProviderOpts,
};

const SYSTEM_PROMPT: &str = "You are a careful assistant with access to tools. \
Use tools when they help. Respond directly when they do not.";

fn main() -> Result<()> {
    let prompt = env::args()
        .nth(1)
        .unwrap_or_else(|| "What time is it? Use the current_time tool.".to_string());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(prompt))
}

async fn run(prompt: String) -> Result<()> {
    let api_key = env::var("OPENROUTER_API_KEY").context("OPENROUTER_API_KEY not set")?;
    let model = env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "anthropic/claude-sonnet-4".to_string());

    let provider: Arc<dyn Provider> = Arc::new(CloudProvider::new(
        api_key,
        "https://openrouter.ai/api/v1",
        "openrouter",
    ));
    let registry = Arc::new(default_registry());
    let sink = Arc::new(LogEventSink);

    println!("=== registered tools ===");
    for n in registry.names() {
        println!("  - {n}");
    }
    println!("\n=== prompt ===\n{prompt}\n");
    println!("=== run ===");

    let mut messages = vec![
        ChatMessage::system(SYSTEM_PROMPT),
        ChatMessage::user(prompt),
    ];
    let opts = AgentOpts {
        provider_opts: ProviderOpts {
            model,
            max_tokens: Some(1024),
            cancel: Arc::new(AtomicBool::new(false)),
        },
        max_steps: 6,
        gate: Arc::new(AutoApproveGate),
    };

    let outcome = run_agent(provider, registry, sink, &mut messages, opts).await?;

    println!("\n=== outcome ===\n{outcome:?}");
    println!("\n=== final message log ({}) ===", messages.len());
    for (i, m) in messages.iter().enumerate() {
        match m {
            ChatMessage::System { content } => println!("  [{i}] system: {}", truncate(content, 80)),
            ChatMessage::User { content } => println!("  [{i}] user: {}", truncate(content, 80)),
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                println!(
                    "  [{i}] assistant: content={} tool_calls={}",
                    truncate(content, 60),
                    tool_calls.len()
                );
                for tc in tool_calls {
                    println!("        - {}({})", tc.name, truncate(&tc.arguments, 80));
                }
            }
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => println!(
                "  [{i}] tool[{}]: {}",
                tool_call_id,
                truncate(content, 80)
            ),
        }
    }

    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.replace('\n', " \\n ")
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push_str("...");
        out.replace('\n', " \\n ")
    }
}

// Force the rezon_lib crate path; otherwise unused-warning.
#[allow(dead_code)]
fn _link() -> Result<()> {
    Err(anyhow!("never called"))
}
