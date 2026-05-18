// Sequential REPL loop.
//
// The shape mirrors classic interactive llm clients (cyllama, ollama
// run): print a prompt, read a line of stdin, either treat it as a
// `/command` or send it as a chat turn. Tokens stream straight to
// stdout; the terminal handles scrollback. The previous turn stays on
// screen indefinitely (until the user scrolls past).
//
// Streaming concurrency: the chat / agent task spawns onto the tokio
// runtime and emits events through `sink::UiEvent` over an mpsc. The
// REPL's `wait_for_turn` drains the channel synchronously, printing
// tokens / tool pills / stats until it sees `Done` or `Error`.
// Ctrl-C cancels the in-flight run by flipping the LlmState /
// AgentRunHandle cancel flag; a second Ctrl-C exits.

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::Arc;

use anstream::{eprintln, print, println};
use anstyle::{AnsiColor, Color, Reset, Style};
use anyhow::Result;
use rezon_core::llm::{self, ChatMsg, ChatOpts, ChatSink, LlmState};
use tokio::signal::ctrl_c;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::agent::{spawn_agent_run, AgentRunHandle};
use crate::input::{self, ReadOutcome, ReplEditor};
use crate::sink::{StatsLite, TuiChatSink, UiEvent};
use crate::store::Store;
use crate::vault::VaultCtx;

// Style palette. Values are `anstyle::Style` / `anstyle::Reset`; both
// types implement `Display` so the existing `{x}...{reset}` format
// strings keep working. Output is routed through `anstream`'s macros,
// which strip SGR codes when stdout isn't a tty (e.g. piped) and
// translate them to Win32 console calls on legacy Windows consoles.
//
// Semantic palette (mirroring the cyllama reference look):
//   cyan    — app name, slash-command names highlighted inline
//   white   — default fg; user input shown bold + bright
//   magenta — per-turn token stats line, tool pills
//   green   — tool-call success marker
//   red     — error marker
//   bright black ("META") — secondary status lines
const fn fg(c: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(c)))
}

const C_RESET: Reset = Reset;
const C_BOLD: Style = Style::new().bold();
const C_DIM: Style = Style::new().dimmed();
const C_USER: Style = fg(AnsiColor::BrightWhite).bold();
const C_APP: Style = fg(AnsiColor::Cyan);
const C_TOOL: Style = fg(AnsiColor::Magenta);
const C_OK: Style = fg(AnsiColor::Green);
const C_ERR: Style = fg(AnsiColor::Red);
const C_META: Style = fg(AnsiColor::BrightBlack);

pub struct Repl {
    state: Arc<LlmState>,
    chat_opts: ChatOpts,
    default_system: String,
    agent_mode: bool,
    max_steps: usize,
    store: Store,
    vault: VaultCtx,
    ui_tx: UnboundedSender<UiEvent>,
    ui_rx: UnboundedReceiver<UiEvent>,
    /// Active agent handle while an agent run is in flight (used for
    /// Ctrl-C cancel). None for chat runs — chat uses LlmState::cancel.
    agent_handle: Option<AgentRunHandle>,
}

impl Repl {
    pub fn new(
        state: LlmState,
        chat_opts: ChatOpts,
        store: Store,
        default_system: String,
        agent_mode: bool,
        max_steps: usize,
        vault: VaultCtx,
    ) -> Self {
        let (ui_tx, ui_rx) = unbounded_channel();
        Self {
            state: Arc::new(state),
            chat_opts,
            default_system,
            agent_mode,
            max_steps,
            store,
            vault,
            ui_tx,
            ui_rx,
            agent_handle: None,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        self.print_banner();
        // Auto-open the most recently used vault on launch.
        if let Some(path) = self.store.active_vault.clone() {
            if let Err(e) = self.vault.open(&path) {
                eprintln!(
                    "{meta}auto-open vault failed: {e}{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
                self.store.active_vault = None;
            } else {
                println!("{meta}vault: {path}{reset}", meta = C_META, reset = C_RESET);
                println!();
            }
        }

        // Set up rustyline editor with persistent history.
        let history_path = input::history_path()?;
        let mut editor: Option<ReplEditor> = Some(input::build_editor(&history_path)?);
        // Pass a plain prompt; `ReplHelper::highlight_prompt` adds
        // the bold-white styling at render time without inflating
        // the width rustyline uses for cursor positioning.
        let prompt = String::from("> ");

        loop {
            let line = match input::read_line(&mut editor, prompt.clone()).await {
                ReadOutcome::Line(s) => s,
                ReadOutcome::Interrupted => continue, // Ctrl-C: new prompt
                ReadOutcome::Eof => break,            // Ctrl-D: exit
            };
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix('/') {
                match self.handle_command(rest).await {
                    CmdResult::Continue => {}
                    CmdResult::Exit => break,
                }
                continue;
            }
            self.run_turn(trimmed.to_string()).await?;
        }
        self.save_ignore_err();
        // Flush history. Errors are non-fatal: the user just loses
        // history for this session.
        if let Some(mut e) = editor {
            let _ = e.save_history(&history_path);
        }
        // Tear down worker threads (embed worker, search watchers,
        // local llama context) in the order documented by their
        // `shutdown` impls. Matters most for local llama: the metal
        // backend asserts that its KV-cache buffers are gone before
        // process-exit destructors run.
        self.vault.shutdown();
        self.state.shutdown();
        Ok(())
    }

    fn print_banner(&self) {
        // Clear the visible viewport but keep scrollback intact —
        // \x1b[2J wipes the screen, \x1b[H homes the cursor; we
        // deliberately don't use \x1b[3J (which would also clear
        // scrollback). Skip when stdout isn't a tty (piped runs).
        if io::stdout().is_terminal() {
            print!("\x1b[2J\x1b[H");
        }
        // App + version: app name in cyan, version in default fg.
        let app = "rezon";
        let version = format!(" v{} chat", env!("CARGO_PKG_VERSION"));
        let model = self.header_model();
        let left_visible_len = app.chars().count() + version.chars().count();
        let width = terminal_size::terminal_size()
            .map(|(terminal_size::Width(w), _)| w as usize)
            .unwrap_or(80);
        let pad = width.saturating_sub(left_visible_len + model.chars().count());
        // `rezon` is bold cyan in the banner; elsewhere `C_APP` stays
        // non-bold so inline command refs (/help, /exit, …) don't
        // shout.
        let banner_app = C_APP.bold();
        println!(
            "{banner_app}{app}{reset}{version}{pad}{model}",
            reset = C_RESET,
            pad = " ".repeat(pad.max(2)),
        );
        println!();
        // Help hint: highlight /help and /exit in cyan, the rest in
        // default fg. Mirrors cyllama's banner style.
        println!(
            "type {app}/help{reset} to list available commands, or {app}/exit{reset} to quit",
            app = C_APP,
            reset = C_RESET,
        );
        if self.agent_mode {
            println!(
                "agent mode is on (use {app}/chat{reset} to disable)",
                app = C_APP,
                reset = C_RESET
            );
        }
        println!();
    }

    fn header_model(&self) -> String {
        if self.chat_opts.provider == "local" {
            if let Some(path) = self.state.status().path {
                return basename(&path);
            }
            return "local".to_string();
        }
        match &self.chat_opts.model {
            Some(m) => format!("{}/{}", self.chat_opts.provider, m),
            None => self.chat_opts.provider.clone(),
        }
    }

    // ---- Turn -----------------------------------------------------

    async fn run_turn(&mut self, user_input: String) -> Result<()> {
        let convo = self.store.active_mut();
        convo.messages.push(ChatMsg {
            role: "user".to_string(),
            content: user_input.clone(),
            ..ChatMsg::default()
        });
        convo.maybe_auto_title();

        if self.agent_mode {
            self.spawn_agent(user_input);
        } else {
            self.spawn_chat();
        }
        self.wait_for_turn().await;
        self.save_ignore_err();
        Ok(())
    }

    fn spawn_chat(&self) {
        let state = self.state.clone();
        let opts = self.chat_opts.clone();
        let msgs = self.store.active().messages.clone();
        let sink: Arc<dyn ChatSink> = Arc::new(TuiChatSink::new(self.ui_tx.clone()));
        let tx = self.ui_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = llm::chat(&state, msgs, opts, sink).await {
                let _ = tx.send(UiEvent::Error(e));
            }
        });
    }

    fn spawn_agent(&mut self, user_input: String) {
        // Pull the just-pushed user message off so the agent path
        // can prepend it itself (build_agent_messages handles that).
        let mut history = self.store.active().messages.clone();
        if matches!(history.last(), Some(m) if m.role == "user") {
            history.pop();
        }
        // `search_notes` is only registered when a vault is open;
        // otherwise the tool wouldn't have anywhere to search.
        let vault_arg = self.vault.active_vault().is_some().then_some(&self.vault);
        match spawn_agent_run(
            self.state.clone(),
            self.chat_opts.clone(),
            history,
            user_input,
            self.ui_tx.clone(),
            self.max_steps,
            vault_arg,
            &self.store.disabled_tools,
        ) {
            Ok(h) => self.agent_handle = Some(h),
            Err(e) => {
                let _ = self.ui_tx.send(UiEvent::Error(e.to_string()));
            }
        }
    }

    /// Drain events until Done / Error. Handles tool confirmation
    /// inline by pausing to read y/n from stdin.
    async fn wait_for_turn(&mut self) {
        let mut assistant = String::new();
        let mut stats: Option<StatsLite> = None;
        let mut interrupts = 0u8;
        let mut newline_pending = false;
        loop {
            tokio::select! {
                ev = self.ui_rx.recv() => {
                    let Some(ev) = ev else { break; };
                    match ev {
                        UiEvent::Token(s) => {
                            print!("{s}");
                            anstream::stdout().flush().ok();
                            assistant.push_str(&s);
                            newline_pending = !s.ends_with('\n');
                        }
                        UiEvent::Stats(s) => stats = Some(s),
                        UiEvent::Done => {
                            if newline_pending { println!(); }
                            // Chat mode: replace the raw streamed
                            // text with markdown-formatted output.
                            // Agent mode keeps the raw stream
                            // because tool pills are interleaved
                            // with assistant tokens — we'd clobber
                            // them.
                            if self.agent_handle.is_none() && !assistant.is_empty() {
                                rerender_markdown(&assistant);
                            }
                            if let Some(s) = stats {
                                print_stats(&s);
                            }
                            println!();
                            break;
                        }
                        UiEvent::Error(e) => {
                            if newline_pending { println!(); newline_pending = false; }
                            eprintln!("{err}error:{reset} {e}", err = C_ERR, reset = C_RESET);
                            println!();
                            // Agent runs always emit `AgentHistory`
                            // + `Done` from the spawn block, even
                            // on error / cancel — keep draining so
                            // the snapshot lands. Chat runs have no
                            // such follow-up, so break immediately.
                            if self.agent_handle.is_none() {
                                break;
                            }
                        }
                        UiEvent::ToolStart { name } => {
                            if newline_pending { println!(); newline_pending = false; }
                            println!("{tool}→ {name}{reset}", tool = C_TOOL, reset = C_RESET);
                        }
                        UiEvent::ToolEnd { ok, summary } => {
                            let icon = if ok { format!("{ok_}✓{reset}", ok_ = C_OK, reset = C_RESET) }
                                       else { format!("{err}✗{reset}", err = C_ERR, reset = C_RESET) };
                            println!("  {icon} {dim}{summary}{reset}", dim = C_DIM, reset = C_RESET);
                        }
                        UiEvent::Confirm { name, arguments, tx } => {
                            if newline_pending { println!(); newline_pending = false; }
                            let approved = prompt_yes_no(&name, &arguments).await;
                            let _ = tx.send(approved);
                        }
                        UiEvent::AgentHistory(msgs) => {
                            // Replace the incrementally-built UI-pill
                            // representation with the agent loop's
                            // structured history (assistant turns with
                            // `tool_calls`, tool-role replies with
                            // `tool_call_id`). Next agent run will
                            // replay these so the model sees its own
                            // prior tool selections.
                            self.store.active_mut().messages = msgs;
                            // The `assistant` accumulator was used to
                            // mirror the streamed assistant text into
                            // a chat-style ChatMsg at end of turn;
                            // the snapshot replaces it, so wipe to
                            // prevent the duplicate push below.
                            assistant.clear();
                        }
                    }
                }
                _ = ctrl_c() => {
                    interrupts += 1;
                    if interrupts == 1 {
                        if let Some(h) = &self.agent_handle {
                            h.cancel();
                        } else {
                            self.state.cancel();
                        }
                        eprintln!("\n{meta}cancelling… (Ctrl-C again to force exit){reset}",
                                  meta = C_META, reset = C_RESET);
                    } else {
                        std::process::exit(130);
                    }
                }
            }
        }
        if !assistant.is_empty() {
            self.store.active_mut().messages.push(ChatMsg {
                role: "assistant".to_string(),
                content: assistant,
                ..ChatMsg::default()
            });
        }
        self.agent_handle = None;
    }

    // ---- Commands -------------------------------------------------

    async fn handle_command(&mut self, line: &str) -> CmdResult {
        let (verb, args) = match line.split_once(char::is_whitespace) {
            Some((v, r)) => (v, r.trim()),
            None => (line.trim(), ""),
        };
        match verb {
            "help" | "h" | "?" => self.cmd_help(),
            "exit" | "quit" | "q" => return CmdResult::Exit,
            "new" => self.cmd_new(),
            "conv" | "c" => self.cmd_conv(args),
            "next" => self.cmd_cycle(1),
            "prev" => self.cmd_cycle(-1),
            "rename" => self.cmd_rename(args),
            "delete" | "del" => self.cmd_delete(),
            "agent" => {
                self.agent_mode = true;
                println!("{meta}agent mode on{reset}", meta = C_META, reset = C_RESET);
            }
            "chat" => {
                self.agent_mode = false;
                println!("{meta}chat mode{reset}", meta = C_META, reset = C_RESET);
            }
            "model" => self.cmd_model(args),
            "provider" => self.cmd_provider(args),
            "max-steps" => self.cmd_max_steps(args),
            "system" => self.cmd_system(args).await,
            "load" => self.cmd_load(args).await,
            "history" => self.cmd_history(),
            "clear" => {
                if io::stdout().is_terminal() {
                    print!("\x1b[2J\x1b[H");
                }
            }
            "vault" => self.cmd_vault(args),
            "note" => self.cmd_note(args),
            "find" => self.cmd_find(args),
            "embed" => self.cmd_embed(args).await,
            "search" => self.cmd_search(args),
            "tools" => self.cmd_tools(args),
            "" => {}
            other => {
                println!(
                    "{err}unknown command:{reset} /{other}",
                    err = C_ERR,
                    reset = C_RESET
                );
            }
        }
        CmdResult::Continue
    }

    fn cmd_help(&self) {
        // Each row is rendered via `help_row` so the verb (and any
        // inline command refs in the description) is highlighted in
        // cyan while the column alignment is preserved.
        println!("commands");
        help_row("/help", "this list");
        help_row("/exit", "quit");
        help_row("/new", "start a new conversation");
        help_row("/conv", "fuzzy picker over conversations");
        help_row("/conv list", "plain numbered listing");
        help_row("/conv <n>", "switch to conversation n (1-indexed)");
        help_row("/next /prev", "cycle conversations");
        help_row("/rename <title>", "rename current conversation");
        help_row("/delete", "delete current conversation");
        help_row("/agent /chat", "toggle agent loop");
        help_row("/model <name>", "change model");
        help_row("/provider <key>", "change provider");
        help_row(
            "/max-steps <n>",
            &format!("agent step cap (current: {})", self.max_steps),
        );
        help_row("/system [text]", "set system prompt (no arg shows current)");
        help_row("/load <gguf>", "load local model in-session");
        help_row("/history", "show current conversation history");
        help_row(
            "/search [query]",
            "fuzzy picker over all conversation messages",
        );
        help_row("/tools", "list tools (✓ enabled · · disabled)");
        help_row(
            "/tools disable <name>",
            "drop a tool from the agent registry",
        );
        help_row(
            "/tools enable <name>",
            "re-enable a previously disabled tool",
        );
        help_row("/clear", "clear the screen");
        println!();
        println!("vault");
        help_row("/vault", "show open vault");
        help_row(
            "/vault <path>",
            "open a vault directory (auto-opens next launch)",
        );
        help_row("/vault close", "forget the saved vault path");
        help_row(
            "/note <path>",
            "read a note from the vault (relative or absolute)",
        );
        help_row(
            "/find <query>",
            "search notes (semantic if /embed loaded, FTS5 otherwise)",
        );
        help_row("/embed", "show embedding model status");
        help_row("/embed <gguf>", "load a local embedding model");
    }

    fn cmd_new(&mut self) {
        self.store.new_conversation(&self.default_system);
        self.save_ignore_err();
        println!(
            "{meta}new conversation{reset}",
            meta = C_META,
            reset = C_RESET
        );
    }

    fn cmd_conv(&mut self, args: &str) {
        if args == "list" {
            for (i, c) in self.store.conversations.iter().enumerate() {
                let marker = if i == self.store.active { "*" } else { " " };
                println!(" {marker} {idx:>2}  {title}", idx = i + 1, title = c.title,);
            }
            return;
        }
        if args.is_empty() {
            // Fuzzy picker over conversation titles. Pre-seed with
            // the current active title so just-pressing-Enter keeps
            // you on the same conversation.
            let items: Vec<String> = self
                .store
                .conversations
                .iter()
                .map(|c| c.title.clone())
                .collect();
            if let Some(idx) = crate::picker::pick(items, "conv ", "") {
                self.store.select(idx);
                self.save_ignore_err();
                println!(
                    "{meta}-> {title}{reset}",
                    meta = C_META,
                    reset = C_RESET,
                    title = self.store.active().title
                );
            }
            return;
        }
        match args.parse::<usize>() {
            Ok(n) if n >= 1 && n <= self.store.conversations.len() => {
                self.store.select(n - 1);
                self.save_ignore_err();
                println!(
                    "{meta}-> {title}{reset}",
                    meta = C_META,
                    reset = C_RESET,
                    title = self.store.active().title
                );
            }
            _ => println!(
                "{err}usage: /conv <1..{}> | /conv (list){reset}",
                self.store.conversations.len(),
                err = C_ERR,
                reset = C_RESET,
            ),
        }
    }

    fn cmd_cycle(&mut self, delta: i32) {
        let n = self.store.conversations.len() as i32;
        if n <= 1 {
            return;
        }
        let i = ((self.store.active as i32 + delta) % n + n) % n;
        self.store.select(i as usize);
        self.save_ignore_err();
        println!(
            "{meta}-> {title}{reset}",
            meta = C_META,
            reset = C_RESET,
            title = self.store.active().title
        );
    }

    fn cmd_rename(&mut self, args: &str) {
        if args.is_empty() {
            println!(
                "{err}usage: /rename <title>{reset}",
                err = C_ERR,
                reset = C_RESET
            );
            return;
        }
        self.store.rename_active(args.to_string());
        self.save_ignore_err();
    }

    fn cmd_delete(&mut self) {
        let sys = self.default_system.clone();
        self.store.delete_active(&sys);
        self.save_ignore_err();
        println!("{meta}deleted{reset}", meta = C_META, reset = C_RESET);
    }

    fn cmd_model(&mut self, args: &str) {
        if args.is_empty() {
            println!(
                "{meta}current model: {}{reset}",
                self.chat_opts
                    .model
                    .clone()
                    .unwrap_or_else(|| "<default>".to_string()),
                meta = C_META,
                reset = C_RESET,
            );
            return;
        }
        self.chat_opts.model = Some(args.to_string());
        println!(
            "{meta}model -> {args}{reset}",
            meta = C_META,
            reset = C_RESET
        );
    }

    fn cmd_provider(&mut self, args: &str) {
        if args.is_empty() {
            println!(
                "{meta}current provider: {}{reset}",
                self.chat_opts.provider,
                meta = C_META,
                reset = C_RESET
            );
            return;
        }
        self.chat_opts.provider = args.to_string();
        println!(
            "{meta}provider -> {args}{reset}",
            meta = C_META,
            reset = C_RESET
        );
    }

    fn cmd_max_steps(&mut self, args: &str) {
        match args.parse::<usize>() {
            Ok(n) if n > 0 => {
                self.max_steps = n;
                println!(
                    "{meta}max-steps -> {n}{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
            }
            _ => println!(
                "{err}usage: /max-steps <positive integer>{reset}",
                err = C_ERR,
                reset = C_RESET
            ),
        }
    }

    async fn cmd_system(&mut self, args: &str) {
        if args.is_empty() {
            let current = self.store.active().system.clone();
            if current.trim().is_empty() {
                println!(
                    "{meta}no system prompt set for this conversation{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
            } else {
                println!(
                    "{meta}current system prompt:{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
                for line in current.lines() {
                    println!("  {line}");
                }
            }
            return;
        }
        self.set_active_system(args.to_string());
        self.save_ignore_err();
        println!(
            "{meta}system prompt updated{reset}",
            meta = C_META,
            reset = C_RESET
        );
    }

    async fn cmd_load(&self, args: &str) {
        if args.is_empty() {
            println!(
                "{err}usage: /load <gguf path>{reset}",
                err = C_ERR,
                reset = C_RESET
            );
            return;
        }
        let label = format!("loading {}", basename(args));
        let result = crate::spinner::with_spinner(label, self.state.load(args.to_string())).await;
        match result {
            Ok(_) => println!(
                "{meta}local model loaded{reset}",
                meta = C_META,
                reset = C_RESET
            ),
            Err(e) => println!("{err}load: {e}{reset}", err = C_ERR, reset = C_RESET),
        }
    }

    fn cmd_tools(&mut self, args: &str) {
        // Mirror the registry assembly in `spawn_agent_run` so the
        // list reflects what the agent actually sees.
        use rezon_core::agent::{
            tool::ToolRegistry,
            tools::{register_core_tools, register_search_notes},
        };
        let mut reg = ToolRegistry::new();
        register_core_tools(&mut reg);
        if self.vault.active_vault().is_some() {
            register_search_notes(
                &mut reg,
                self.vault.search.clone(),
                self.vault.embed.clone(),
            );
        }
        let all: Vec<String> = reg.names().map(str::to_string).collect();

        let (verb, name) = match args.split_once(char::is_whitespace) {
            Some((v, n)) => (v.trim(), n.trim()),
            None => (args.trim(), ""),
        };
        match verb {
            "" | "list" => {
                for n in &all {
                    let disabled = self.store.disabled_tools.iter().any(|d| d == n);
                    let (marker, style) = if disabled {
                        ("·", C_DIM)
                    } else {
                        ("✓", C_OK)
                    };
                    let _ = style;
                    println!(
                        "  {style}{marker}{reset}  {n}",
                        marker = marker,
                        style = if disabled { C_DIM } else { C_OK },
                        reset = C_RESET,
                    );
                }
                if all.is_empty() {
                    println!(
                        "{meta}no tools registered{reset}",
                        meta = C_META,
                        reset = C_RESET
                    );
                }
            }
            "disable" => {
                let pick_name = if name.is_empty() {
                    let enabled: Vec<String> = all
                        .iter()
                        .filter(|n| !self.store.disabled_tools.iter().any(|d| d == *n))
                        .cloned()
                        .collect();
                    if enabled.is_empty() {
                        println!(
                            "{meta}all tools already disabled{reset}",
                            meta = C_META,
                            reset = C_RESET
                        );
                        return;
                    }
                    match crate::picker::pick(enabled.clone(), "disable ", "") {
                        Some(i) => enabled[i].clone(),
                        None => return,
                    }
                } else if all.iter().any(|n| n == name) {
                    name.to_string()
                } else {
                    println!(
                        "{err}unknown tool: {name}{reset}",
                        err = C_ERR,
                        reset = C_RESET
                    );
                    return;
                };
                if !self.store.disabled_tools.iter().any(|d| d == &pick_name) {
                    self.store.disabled_tools.push(pick_name.clone());
                    self.save_ignore_err();
                }
                println!(
                    "{meta}disabled: {pick_name}{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
            }
            "enable" => {
                let pick_name = if name.is_empty() {
                    let disabled = self.store.disabled_tools.clone();
                    if disabled.is_empty() {
                        println!(
                            "{meta}no disabled tools{reset}",
                            meta = C_META,
                            reset = C_RESET
                        );
                        return;
                    }
                    match crate::picker::pick(disabled.clone(), "enable ", "") {
                        Some(i) => disabled[i].clone(),
                        None => return,
                    }
                } else {
                    name.to_string()
                };
                let before = self.store.disabled_tools.len();
                self.store.disabled_tools.retain(|d| d != &pick_name);
                if self.store.disabled_tools.len() != before {
                    self.save_ignore_err();
                }
                println!(
                    "{meta}enabled: {pick_name}{reset}",
                    meta = C_META,
                    reset = C_RESET
                );
            }
            other => {
                println!(
                    "{err}unknown subcommand: /tools {other}{reset}",
                    err = C_ERR,
                    reset = C_RESET
                );
            }
        }
    }

    fn cmd_search(&mut self, args: &str) {
        // Build candidates: one entry per non-system / non-tool
        // message, tagged with `(conv_idx, msg_idx)` so a pick can
        // route us back to the right conversation. The display
        // string is what the matcher sees — keep it human-readable
        // (you / rzn marker + first-line snippet + conv title).
        struct Candidate {
            display: String,
            conv_idx: usize,
            msg_idx: usize,
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        for (ci, conv) in self.store.conversations.iter().enumerate() {
            for (mi, m) in conv.messages.iter().enumerate() {
                if matches!(m.role.as_str(), "system" | "tool") {
                    continue;
                }
                let role_label = match m.role.as_str() {
                    "user" => "you",
                    "assistant" => "rzn",
                    other => other,
                };
                let first_line = m.content.lines().next().unwrap_or("");
                let snippet: String = first_line.chars().take(80).collect();
                candidates.push(Candidate {
                    display: format!("{role_label} · {snippet}  ({})", conv.title),
                    conv_idx: ci,
                    msg_idx: mi,
                });
            }
        }
        if candidates.is_empty() {
            println!(
                "{meta}no messages to search{reset}",
                meta = C_META,
                reset = C_RESET
            );
            return;
        }
        let items: Vec<String> = candidates.iter().map(|c| c.display.clone()).collect();
        let Some(idx) = crate::picker::pick(items, "search ", args) else {
            return;
        };
        let pick = &candidates[idx];
        // Switch to the picked conversation and print the message
        // body so the user sees the match in context.
        if self.store.active != pick.conv_idx {
            self.store.select(pick.conv_idx);
            self.save_ignore_err();
        }
        let conv = self.store.active();
        println!(
            "{meta}-> {title}{reset}",
            meta = C_META,
            reset = C_RESET,
            title = conv.title
        );
        println!();
        let msg = &conv.messages[pick.msg_idx];
        let role_label = match msg.role.as_str() {
            "user" => "you",
            "assistant" => "rzn",
            other => other,
        };
        println!("{meta}{role_label}:{reset}", meta = C_META, reset = C_RESET);
        for line in msg.content.lines() {
            println!("  {line}");
        }
        println!();
    }

    #[allow(dead_code)]
    fn cmd_search_substring(&self, args: &str) {
        let q_lc = args.to_lowercase();
        // Cap output so a popular term doesn't bury the terminal.
        const MAX_LINES_PER_CONV: usize = 6;
        const MAX_TOTAL_LINES: usize = 80;
        let mut total_lines = 0usize;
        let mut total_convs = 0usize;
        let mut total_messages = 0usize;

        // Iterate newest-first so recent matches surface at the top
        // of the output buffer.
        for (i, conv) in self.store.conversations.iter().enumerate().rev() {
            // Collect (msg_index, &ChatMsg) hits inside this conv.
            let hits: Vec<&ChatMsg> = conv
                .messages
                .iter()
                .filter(|m| m.content.to_lowercase().contains(&q_lc))
                .collect();
            if hits.is_empty() {
                continue;
            }
            total_messages += hits.len();
            let marker = if i == self.store.active { "*" } else { " " };
            println!(
                "{meta}{marker} conv {idx:>2} · {title}{reset}",
                marker = marker,
                idx = i + 1,
                title = conv.title,
                meta = C_META,
                reset = C_RESET,
            );
            let mut conv_lines = 0usize;
            'msg: for m in hits {
                let role_label = match m.role.as_str() {
                    "user" => "you",
                    "assistant" => "rzn",
                    "system" => "sys",
                    "tool" => "tool",
                    other => other,
                };
                for raw_line in m.content.lines() {
                    if !raw_line.to_lowercase().contains(&q_lc) {
                        continue;
                    }
                    let highlighted = highlight_match(raw_line, args);
                    println!(
                        "    {meta}{role:>4}{reset}  {highlighted}",
                        role = role_label,
                        meta = C_META,
                        reset = C_RESET,
                    );
                    conv_lines += 1;
                    total_lines += 1;
                    if conv_lines >= MAX_LINES_PER_CONV {
                        println!("    {meta}…{reset}", meta = C_META, reset = C_RESET);
                        break 'msg;
                    }
                    if total_lines >= MAX_TOTAL_LINES {
                        break 'msg;
                    }
                }
            }
            println!();
            total_convs += 1;
            if total_lines >= MAX_TOTAL_LINES {
                println!(
                    "{meta}(output capped at {} lines · refine the query for more){reset}",
                    MAX_TOTAL_LINES,
                    meta = C_META,
                    reset = C_RESET,
                );
                break;
            }
        }
        if total_convs == 0 {
            println!("{meta}no matches{reset}", meta = C_META, reset = C_RESET);
        } else {
            println!(
                "{meta}{m} match{mp} across {c} conversation{cp}{reset}",
                m = total_messages,
                mp = if total_messages == 1 { "" } else { "es" },
                c = total_convs,
                cp = if total_convs == 1 { "" } else { "s" },
                meta = C_META,
                reset = C_RESET,
            );
        }
    }

    fn cmd_vault(&mut self, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            match self.vault.active_vault() {
                Some(p) => println!("{meta}vault: {p}{reset}", meta = C_META, reset = C_RESET),
                None => println!(
                    "{meta}no vault open · use {app}/vault <path>{reset}",
                    meta = C_META,
                    app = C_APP,
                    reset = C_RESET
                ),
            }
            return;
        }
        if args == "close" {
            let was_open = self.vault.close();
            self.store.active_vault = None;
            self.save_ignore_err();
            if was_open {
                println!("{meta}vault closed{reset}", meta = C_META, reset = C_RESET);
            } else {
                println!(
                    "{meta}no vault to close (forgot persisted path){reset}",
                    meta = C_META,
                    reset = C_RESET
                );
            }
            return;
        }
        match self.vault.open(args) {
            Ok(_) => {
                let active = self.vault.active_vault();
                self.store.active_vault = active.clone();
                self.save_ignore_err();
                println!(
                    "{meta}vault: {}{reset}",
                    active.as_deref().unwrap_or(args),
                    meta = C_META,
                    reset = C_RESET
                );
            }
            Err(e) => {
                println!("{err}vault: {e}{reset}", err = C_ERR, reset = C_RESET);
            }
        }
    }

    fn cmd_note(&self, args: &str) {
        if args.is_empty() {
            println!(
                "{err}usage: /note <path>{reset}",
                err = C_ERR,
                reset = C_RESET
            );
            return;
        }
        match self.vault.read_note(args) {
            Ok(text) => {
                println!("{}", text);
            }
            Err(e) => println!("{err}note: {e}{reset}", err = C_ERR, reset = C_RESET),
        }
    }

    fn cmd_find(&self, args: &str) {
        if args.is_empty() {
            println!(
                "{err}usage: /find <query>{reset}",
                err = C_ERR,
                reset = C_RESET
            );
            return;
        }
        match self.vault.find(args, 8) {
            Ok((hits, mode)) => {
                if hits.is_empty() {
                    println!(
                        "{meta}no matches ({mode}){reset}",
                        meta = C_META,
                        reset = C_RESET
                    );
                    return;
                }
                println!(
                    "{meta}{n} match{plural} ({mode}){reset}",
                    meta = C_META,
                    reset = C_RESET,
                    n = hits.len(),
                    plural = if hits.len() == 1 { "" } else { "es" },
                );
                for h in hits {
                    println!("{app}{}{reset}", h.path, app = C_APP, reset = C_RESET);
                    for line in h.snippet.lines() {
                        println!("  {dim}{line}{reset}", dim = C_DIM, reset = C_RESET);
                    }
                }
            }
            Err(e) => println!("{err}find: {e}{reset}", err = C_ERR, reset = C_RESET),
        }
    }

    async fn cmd_embed(&self, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            let s = self.vault.embed_status();
            if s.loaded {
                println!(
                    "{meta}embed: {} (dim={}){reset}",
                    s.path.unwrap_or_default(),
                    s.dim.unwrap_or(0),
                    meta = C_META,
                    reset = C_RESET
                );
            } else {
                println!(
                    "{meta}no embed model loaded · use {app}/embed <gguf>{reset}",
                    meta = C_META,
                    app = C_APP,
                    reset = C_RESET
                );
            }
            return;
        }
        let label = format!("loading {}", basename(args));
        let result =
            crate::spinner::with_spinner(label, self.vault.load_embed(args.to_string())).await;
        match result {
            Ok(s) => println!(
                "{meta}embed: loaded (dim={}){reset}",
                s.dim.unwrap_or(0),
                meta = C_META,
                reset = C_RESET
            ),
            Err(e) => println!("{err}embed: {e}{reset}", err = C_ERR, reset = C_RESET),
        }
    }

    fn cmd_history(&self) {
        for m in &self.store.active().messages {
            match m.role.as_str() {
                "user" => println!(
                    "{user}> {}{reset}",
                    m.content,
                    user = C_USER,
                    reset = C_RESET
                ),
                "assistant" => println!("{}", m.content),
                "system" => println!(
                    "{dim}[system] {}{reset}",
                    m.content,
                    dim = C_DIM,
                    reset = C_RESET
                ),
                "tool" => println!("{tool}{}{reset}", m.content, tool = C_TOOL, reset = C_RESET),
                _ => println!("[{role}] {}", m.content, role = m.role),
            }
            println!();
        }
    }

    fn set_active_system(&mut self, text: String) {
        let convo = self.store.active_mut();
        convo.system = text.clone();
        let has_system = matches!(convo.messages.first(), Some(m) if m.role == "system");
        if text.trim().is_empty() {
            if has_system {
                convo.messages.remove(0);
            }
        } else if has_system {
            convo.messages[0].content = text;
        } else {
            convo.messages.insert(
                0,
                ChatMsg {
                    role: "system".to_string(),
                    content: text,
                    ..ChatMsg::default()
                },
            );
        }
    }

    fn save_ignore_err(&self) {
        if let Err(e) = self.store.save() {
            eprintln!("save store: {e}");
        }
    }
}

enum CmdResult {
    Continue,
    Exit,
}

/// Render a single `/help` row: the verb segment in cyan, the
/// description in default fg. Splits on the first whitespace so a
/// trailing `<arg>` or `[arg]` syntax stays in cyan along with the
/// slash command, but multi-word usages like `/next /prev` still
/// render both verbs in cyan.
fn help_row(usage: &str, description: &str) {
    // We word-split and colour each `/`-prefixed token cyan, others
    // (placeholders like `<n>`, `[text]`, …) in default fg. Width
    // is computed on the *plain* text so the SGR codes don't shift
    // the description column.
    const COLUMN: usize = 20;
    let plain_width = usage.chars().count();
    let pad = COLUMN.saturating_sub(plain_width);
    let mut rendered = String::with_capacity(usage.len() + 16);
    for (i, token) in usage.split(' ').enumerate() {
        if i > 0 {
            rendered.push(' ');
        }
        if token.starts_with('/') {
            rendered.push_str(&format!("{C_APP}{token}{C_APP:#}"));
        } else {
            rendered.push_str(token);
        }
    }
    println!("  {rendered}{pad}{description}", pad = " ".repeat(pad));
}

/// Overwrite the raw streamed assistant text with a markdown-
/// formatted version. Counts the rows the raw text occupied, scrolls
/// the cursor back to the top of that block, clears to end-of-screen,
/// and writes the rendered output in place. Silently no-ops when
/// stdout isn't a tty (piped output keeps the raw markdown — still
/// human-readable, and avoids spewing cursor-control escapes into
/// pipes that may not strip them).
fn rerender_markdown(raw: &str) {
    use std::io::Write;
    if !std::io::stdout().is_terminal() {
        return;
    }
    let width = terminal_size::terminal_size()
        .map(|(terminal_size::Width(w), _)| w)
        .unwrap_or(80);
    let rows = crate::markdown::count_rows(raw, width);
    if rows == 0 {
        return;
    }
    let formatted = crate::markdown::render(raw);
    let mut out = anstream::stdout().lock();
    // \x1b[<n>A moves the cursor up n rows. \r returns to col 0.
    // \x1b[J clears from the cursor to the end of the screen.
    let _ = write!(out, "\x1b[{rows}A\r\x1b[J");
    let _ = out.write_all(formatted.as_bytes());
    let _ = out.flush();
}

/// Inline-highlight every case-insensitive occurrence of `query` in
/// `line` with bold bright yellow. The result is a plain String with
/// embedded SGR escapes; anstream strips them when stdout isn't a
/// tty.
fn highlight_match(line: &str, query: &str) -> String {
    let line_lc = line.to_lowercase();
    let query_lc = query.to_lowercase();
    let q_len = query.len();
    let mut out = String::with_capacity(line.len() + 16);
    let mut cursor = 0;
    while cursor < line.len() {
        match line_lc[cursor..].find(&query_lc) {
            Some(rel) => {
                let start = cursor + rel;
                let end = start + q_len;
                out.push_str(&line[cursor..start]);
                // Hand-rolled escape so this fn doesn't need to take
                // the anstyle Style consts; rustfmt + anstream
                // friendly.
                out.push_str("\x1b[1;93m");
                out.push_str(&line[start..end]);
                out.push_str("\x1b[0m");
                cursor = end;
            }
            None => {
                out.push_str(&line[cursor..]);
                break;
            }
        }
    }
    out
}

fn basename(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(p)
        .to_string()
}

fn print_stats(s: &StatsLite) {
    let prompt = s
        .prompt_tokens
        .map(|n| n.to_string())
        .unwrap_or_else(|| "?".into());
    let secs = (s.duration_ms as f64) / 1000.0;
    let tps = if secs > 0.0 {
        (s.gen_tokens as f64) / secs
    } else {
        0.0
    };
    println!(
        "{stats}[ Prompt: {prompt} tok | Generation: {gen} tok @ {tps:.1} t/s ]{reset}",
        stats = C_TOOL,
        reset = C_RESET,
        gen = s.gen_tokens,
    );
}

async fn prompt_yes_no(name: &str, arguments: &str) -> bool {
    let prompt = format!(
        "{tool}approve tool{reset} {bold}{name}{reset} {dim}with args{reset} {arguments}\n{user}[y/N] > ",
        tool = C_TOOL,
        reset = C_RESET,
        bold = C_BOLD,
        user = C_USER,
        dim = C_DIM,
    );
    let resp = tokio::task::spawn_blocking(move || {
        print!("{prompt}");
        anstream::stdout().flush().ok();
        let mut buf = String::new();
        let _ = io::stdin().lock().read_line(&mut buf);
        print!("{C_RESET}");
        anstream::stdout().flush().ok();
        buf
    })
    .await
    .unwrap_or_default();
    matches!(resp.trim().to_lowercase().as_str(), "y" | "yes")
}
