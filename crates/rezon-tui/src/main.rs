// rezon-tui: terminal chat shell over `rezon-core`.
//
// Sequential REPL — no ratatui, no alternate screen, no modes.
// Output streams to stdout as it arrives; terminal scrollback keeps
// the conversation history. Slash commands handle navigation and
// configuration; `/help` lists them.

mod agent;
mod input;
mod markdown;
mod picker;
mod repl;
mod sink;
mod spinner;
mod store;
mod vault;

use anyhow::{Context, Result};
use clap::Parser;
use rezon_core::llm::{ChatOpts, LlmState};

use crate::repl::Repl;

#[derive(Debug, Parser)]
#[command(
    name = "rezon-tui",
    about = "Sequential chat REPL over rezon-core (local llama.cpp or OpenAI-compatible cloud providers)"
)]
struct Cli {
    /// Provider key: "local", "openai", "anthropic", "openrouter", or "other".
    #[arg(long, default_value = "openrouter")]
    provider: String,

    /// Model identifier. For cloud providers this is the model slug
    /// (e.g. "anthropic/claude-sonnet-4"); for `--provider local` it's
    /// the path to a GGUF file.
    #[arg(long)]
    model: Option<String>,

    /// Path to a GGUF file. Alias for `--model` when
    /// `--provider local` is in use; ignored otherwise.
    #[arg(long)]
    gguf: Option<String>,

    /// Base URL for `--provider other` (e.g. http://localhost:11434/v1
    /// for Ollama).
    #[arg(long)]
    base_url: Option<String>,

    /// Override the provider's API-key env var.
    #[arg(long)]
    api_key: Option<String>,

    /// System prompt prepended to new conversations.
    #[arg(long)]
    system: Option<String>,

    /// Run the multi-step agent loop (tool use). Toggle in-session
    /// with `/agent` and `/chat`.
    #[arg(long)]
    agent: bool,

    /// Hard cap on agent loop iterations. Override in-session with
    /// `/max-steps <n>`.
    #[arg(long, default_value_t = 8)]
    max_steps: usize,

    /// Show agent reasoning ("thinking") blocks as they stream.
    /// Toggle per-conversation with `/thinking on|off|toggle`.
    #[arg(long)]
    show_thinking: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    // sqlite-vec needs to be registered before any rusqlite
    // Connection::open call; do it once at process start.
    rezon_core::search::register_sqlite_vec();

    let state = LlmState::default();

    let local_path = match cli.provider.as_str() {
        "local" => Some(
            cli.gguf
                .clone()
                .or_else(|| cli.model.clone())
                .context("--provider local requires --gguf or --model <path-to-gguf>")?,
        ),
        _ => None,
    };
    if let Some(path) = local_path.as_ref() {
        let label = format!(
            "loading {}",
            std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
        );
        spinner::with_spinner(label, state.load(path.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("load model: {e}"))?;
    }

    let chat_opts = ChatOpts {
        provider: cli.provider.clone(),
        model: cli.model.clone(),
        base_url: cli.base_url.clone(),
        api_key: cli.api_key.clone(),
    };

    let default_system = cli.system.clone().unwrap_or_default();
    let (store, _existed) = store::Store::load_or_new(&default_system)?;

    let vault = vault::VaultCtx::new()?;
    let mut repl = Repl::new(
        state,
        chat_opts,
        store,
        default_system,
        cli.agent,
        cli.max_steps,
        vault,
        cli.show_thinking,
    );
    repl.run().await
}
