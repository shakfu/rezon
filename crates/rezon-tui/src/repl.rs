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
                eprintln!("{meta}auto-open vault failed: {e}{reset}",
                          meta = C_META, reset = C_RESET);
                self.store.active_vault = None;
            } else {
                println!("{meta}vault: {path}{reset}",
                         meta = C_META, reset = C_RESET);
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
        println!(
            "{app_color}{app}{reset}{version}{pad}{model}",
            app_color = C_APP,
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
            println!("agent mode is on (use {app}/chat{reset} to disable)",
                     app = C_APP, reset = C_RESET);
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
                            if let Some(s) = stats {
                                print_stats(&s);
                            }
                            println!();
                            break;
                        }
                        UiEvent::Error(e) => {
                            if newline_pending { println!(); }
                            eprintln!("{err}error:{reset} {e}", err = C_ERR, reset = C_RESET);
                            println!();
                            break;
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
            "" => {}
            other => {
                println!("{err}unknown command:{reset} /{other}", err = C_ERR, reset = C_RESET);
            }
        }
        CmdResult::Continue
    }

    fn cmd_help(&self) {
        println!("{bold}commands{reset}", bold = C_BOLD, reset = C_RESET);
        println!("  /help               this list");
        println!("  /exit               quit");
        println!("  /new                start a new conversation");
        println!("  /conv               list conversations");
        println!("  /conv <n>           switch to conversation n (1-indexed)");
        println!("  /next /prev         cycle conversations");
        println!("  /rename <title>     rename current conversation");
        println!("  /delete             delete current conversation");
        println!("  /agent /chat        toggle agent loop");
        println!("  /model <name>       change model");
        println!("  /provider <key>     change provider");
        println!("  /max-steps <n>      agent step cap (current: {})", self.max_steps);
        println!("  /system [text]      set system prompt (no arg shows current)");
        println!("  /load <gguf>        load local model in-session");
        println!("  /history            show current conversation history");
        println!("  /search <query>     search across all conversations");
        println!("  /clear              clear the screen");
        println!();
        println!("{bold}vault{reset}", bold = C_BOLD, reset = C_RESET);
        println!("  /vault              show open vault");
        println!("  /vault <path>       open a vault directory (auto-opens next launch)");
        println!("  /vault close        forget the saved vault path");
        println!("  /note <path>        read a note from the vault (relative or absolute)");
        println!("  /find <query>       search notes (semantic if /embed loaded, FTS5 otherwise)");
        println!("  /embed              show embedding model status");
        println!("  /embed <gguf>       load a local embedding model");
    }

    fn cmd_new(&mut self) {
        self.store.new_conversation(&self.default_system);
        self.save_ignore_err();
        println!("{meta}new conversation{reset}", meta = C_META, reset = C_RESET);
    }

    fn cmd_conv(&mut self, args: &str) {
        if args.is_empty() {
            for (i, c) in self.store.conversations.iter().enumerate() {
                let marker = if i == self.store.active { "*" } else { " " };
                println!(
                    " {marker} {idx:>2}  {title}",
                    idx = i + 1,
                    title = c.title,
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
            println!("{err}usage: /rename <title>{reset}", err = C_ERR, reset = C_RESET);
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
        println!("{meta}model -> {args}{reset}", meta = C_META, reset = C_RESET);
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
        println!("{meta}provider -> {args}{reset}", meta = C_META, reset = C_RESET);
    }

    fn cmd_max_steps(&mut self, args: &str) {
        match args.parse::<usize>() {
            Ok(n) if n > 0 => {
                self.max_steps = n;
                println!("{meta}max-steps -> {n}{reset}", meta = C_META, reset = C_RESET);
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
                println!("{meta}no system prompt set for this conversation{reset}",
                         meta = C_META, reset = C_RESET);
            } else {
                println!("{meta}current system prompt:{reset}", meta = C_META, reset = C_RESET);
                for line in current.lines() {
                    println!("  {line}");
                }
            }
            return;
        }
        self.set_active_system(args.to_string());
        self.save_ignore_err();
        println!("{meta}system prompt updated{reset}", meta = C_META, reset = C_RESET);
    }

    async fn cmd_load(&self, args: &str) {
        if args.is_empty() {
            println!("{err}usage: /load <gguf path>{reset}", err = C_ERR, reset = C_RESET);
            return;
        }
        println!("{meta}loading {args}…{reset}", meta = C_META, reset = C_RESET);
        match self.state.load(args.to_string()).await {
            Ok(_) => println!("{meta}local model loaded{reset}", meta = C_META, reset = C_RESET),
            Err(e) => println!("{err}load: {e}{reset}", err = C_ERR, reset = C_RESET),
        }
    }

    fn cmd_search(&self, args: &str) {
        if args.is_empty() {
            println!(
                "{err}usage: /search <query>{reset}",
                err = C_ERR,
                reset = C_RESET
            );
            return;
        }
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
                        println!(
                            "    {meta}…{reset}",
                            meta = C_META,
                            reset = C_RESET
                        );
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
            self.store.active_vault = None;
            self.save_ignore_err();
            println!("{meta}vault closed (process still holds the open index — restart to fully release){reset}",
                     meta = C_META, reset = C_RESET);
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
        println!("{meta}loading {args}…{reset}", meta = C_META, reset = C_RESET);
        match self.vault.load_embed(args.to_string()).await {
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
                "user" => println!("{user}> {}{reset}", m.content, user = C_USER, reset = C_RESET),
                "assistant" => println!("{}", m.content),
                "system" => println!("{dim}[system] {}{reset}", m.content, dim = C_DIM, reset = C_RESET),
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
