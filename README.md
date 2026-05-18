# rezon

LLM client with interchangeable providers. Ships two shells over one
shared Rust backend:

- **`rezon`** — Tauri 2 + React 19 desktop app (`make dev` / `make
  build`).
- **`rezon-tui`** — sequential REPL chat for the terminal
  (`make run-tui`).

Providers:

- **Local**: [`llama-cpp-2`](https://crates.io/crates/llama-cpp-2)
  with Metal acceleration, loading any `.gguf` model from disk.
- **Cloud (OpenAI / Anthropic / OpenRouter)**:
  [`async-openai`](https://github.com/64bit/async-openai) client
  pointed at each provider's OpenAI-compatible endpoint.
- **Other**: same `async-openai` client with model + base URL + API
  key supplied at runtime. Targets any OpenAI-compatible server —
  Ollama, LM Studio, `llama.cpp` `server`, self-hosted gateways, etc.

### Cloud providers

| key          | env var              | base URL                         |
| ------------ | -------------------- | -------------------------------- |
| `openai`     | `OPENAI_API_KEY`     | `https://api.openai.com/v1`      |
| `anthropic`  | `ANTHROPIC_API_KEY`  | `https://api.anthropic.com/v1`   |
| `openrouter` | `OPENROUTER_API_KEY` | `https://openrouter.ai/api/v1`   |
| `other`      | (entered at runtime) | (entered at runtime)             |

Recommended-models lists for the named providers live in
`crates/rezon-core/models.json` (embedded into the binary via
`include_str!`).

## Layout

```
src/                          React + Vite frontend (consumed by rezon-web)
crates/
  rezon-core/                 Provider-agnostic backend (no Tauri deps)
    src/
      llm.rs                  Chat: local llama.cpp + cloud (async-openai).
                              `ChatSink` trait abstracts the event surface.
      embed.rs                Embedding model worker + catch-up loop.
      search.rs               FTS5 + sqlite-vec per-vault index, file watcher.
      vault.rs                Path-traversal-safe filesystem ops.
      agent/                  Provider-agnostic agent loop + tools.
        cloud.rs              Cloud `Provider` impl.
        local.rs              Local `Provider` impl (owns `Arc<LlmState>`).
        loop_.rs              `run_agent` — streaming, tool dispatch.
        tools/                current_time, file_read, shell_exec, web_fetch,
                              search_notes.
        confirm.rs            `ConfirmationGate` trait.
        event.rs              `EventSink` trait + `LogEventSink`.
    models.json               Cloud provider catalog.
  rezon-web/                  Tauri shell — thin wrapper over rezon-core
    src/
      lib.rs                  Builder, command registration, auto-load.
      llm.rs                  `#[tauri::command]` wrappers + `TauriChatSink`.
      embed.rs                Embed lifecycle + event emission.
      search.rs               Search commands.
      vault.rs                Vault commands.
      agent/
        commands.rs           `agent_chat` / `cancel_agent` / `confirm_tool_call`.
        tauri_sink.rs         `EventSink` -> `app.emit("agent-*", …)`.
        tauri_gate.rs         `ConfirmationGate` -> frontend prompt.
    tauri.conf.json           `frontendDist: "../../dist"`.
  rezon-tui/                  Terminal REPL — also thin
    src/
      repl.rs                 Slash-command dispatcher + streaming loop.
      sink.rs                 `TuiChatSink`, `TuiAgentSink`,
                              `TuiConfirmationGate`.
      agent.rs                Builds the agent registry; spawns runs.
      vault.rs                `VaultCtx` (Arc<SearchState> + Arc<EmbedState>).
      input.rs                rustyline editor + tab completion.
      picker.rs               nucleo + crossterm fuzzy picker.
      markdown.rs             Inline markdown -> ANSI renderer.
      spinner.rs              Braille spinner for blocking loads.
      store.rs                Conversations / vault / disabled-tools JSON.
```

The Cargo workspace root is `Cargo.toml`; `make check` / `make test`
cover all three crates.

## Quick start

### GUI (rezon-web)

```sh
make install      # bun install (frontend deps)
make dev          # bun run tauri dev --config crates/rezon-web/tauri.conf.json
```

API keys for cloud providers can be set in the environment or entered
in the right-sidebar Settings; the **Other** provider takes a base URL
+ (optional) API key in the UI.

### TUI (rezon-tui)

```sh
make run-tui ARGS="--provider openrouter --model anthropic/claude-haiku-4-5"
make run-tui ARGS="--provider local --gguf /path/to/model.gguf"
make run-tui-release ARGS="--agent --provider openrouter \
                            --model openai/gpt-5.4-mini"
```

Inside the REPL, `/help` lists slash commands. Conversations,
disabled-tools, last vault, and command history persist under
`~/Library/Application Support/com.rezon.rezon-tui/`.

Key features:

- Streaming responses with per-turn token stats.
- Multiple conversations (`/conv`, `/new`, `/rename`, `/delete`,
  `/next`, `/prev`).
- Fuzzy picker over conversations and search results (`/conv`,
  `/search`).
- Agent mode (`/agent`) with `current_time`, `file_read`, `shell_exec`,
  `web_fetch`, and `search_notes` (when a vault is open). Tool calls
  show inline; confirmation-required tools prompt `[y/N]`.
- Tool turns persist across restarts so the agent sees its own prior
  calls.
- Vault integration: `/vault <path>`, `/note <path>`, `/find <query>`.
  `/embed <gguf>` loads a separate embedding model; semantic search
  takes over once embeddings are caught up.
- Markdown rendering on assistant responses (raw stream during, then
  re-rendered as bold/italic/code/headings/lists once complete).
- rustyline editing: ↑/↓ history, Ctrl-R reverse search, tab
  completion for slash commands + filesystem paths.

## Architecture notes

### `ChatSink` / `EventSink` / `ConfirmationGate`

The boundaries between core and the two shells are three traits in
`rezon-core`:

- `ChatSink` — token / stats / done callbacks for the non-tool chat
  path. `TauriChatSink` forwards to `app.emit("chat-token"|"chat-stats"
  |"chat-done", …)`; `TuiChatSink` pushes into an mpsc the REPL
  drains.
- `EventSink` (agent path) — token, tool start/end, stats, done,
  cancelled, error.
- `ConfirmationGate` — async `ask(call) -> Approved | Denied`.
  `TauriConfirmationGate` round-trips through a `confirm_tool_call`
  command; `TuiConfirmationGate` writes a `UiEvent::Confirm` and
  blocks on a oneshot the REPL fulfils after reading y/n from stdin.

Frontend event names and payloads are unchanged from before the
workspace split — no migration needed for the existing React UI.

### Local model teardown

`LlmState::shutdown` joins the per-model worker thread before
dropping the `Arc<LlamaBackend>`. Without this, ggml-metal's
process-exit destructor (run from `__cxa_finalize` after `main`
returns) trips `GGML_ASSERT([rsets->data count] == 0)` because the
KV-cache buffers are still alive when the metal device is torn down.

Each loaded local model gets a dedicated worker holding the
`LlamaContext` and reusing the KV cache across turns: only the
divergent suffix of each new prompt is decoded.

### Defaults

Hard-coded in `crates/rezon-core/src/llm.rs`:

- `N_CTX = 4096`
- `MAX_NEW_TOKENS = 1024`
- `N_GPU_LAYERS = 999` (offload everything; Metal feature enabled)
- Sampler chain: `temp(0.7)` then `dist(1234)` (fixed seed)

The model's own chat template (`model.chat_template(None)`) is used to
format the prompt; models without embedded chat-template metadata
cannot chat.

## Develop

```
make install            bun install (frontend deps)
make dev                Tauri GUI dev mode
make build              Tauri GUI release build
make web-dev            Vite only, no Tauri shell
make web-build          Frontend only

make build-tui          rezon-tui (debug)
make build-tui-release  rezon-tui (release)
make run-tui ARGS="…"   Run rezon-tui (debug), forwarding ARGS to the binary
make run-tui-release ARGS="…"

make check              cargo check --workspace
make fmt                cargo fmt --all
make fmt-check          cargo fmt --all -- --check
make lint               cargo clippy --workspace --all-targets -- -D warnings
make test               cargo test --workspace
make clean              rm node_modules dist target …
```

## Requirements

- macOS with Metal for local models (other platforms untested for
  Metal; the cloud providers and `rezon-tui` should build anywhere
  Rust + Tauri prerequisites are available).
- Rust toolchain.
- Bun (frontend deps + Tauri CLI). Only needed for the GUI; `rezon-tui`
  builds with `cargo` alone.
- Tauri 2 prerequisites for the GUI.
- For the local backend: a GGUF model file with chat-template metadata.
- For named cloud backends: the relevant API key (`OPENAI_API_KEY` /
  `ANTHROPIC_API_KEY` / `OPENROUTER_API_KEY`) in the environment when
  launching, or supplied at runtime via the GUI's settings or
  `rezon-tui`'s `--api-key`.

See `CHANGELOG.md` for history.
