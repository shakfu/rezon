# Changelog

All notable changes to this project. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- **`rezon-tui` crate** — sequential REPL chat shell over `rezon-core`,
  shipped as the `rezon-tui` binary. Uses plain stdin/stdout (no
  alternate-screen takeover) so terminal scrollback / copy-paste /
  piping all work. clap CLI mirrors the GUI's provider/model knobs:
  `--provider {local,openai,anthropic,openrouter,other}`, `--model`,
  `--gguf`, `--base-url`, `--api-key`, `--system`, `--agent`,
  `--max-steps`.
- **Slash-command surface** in `rezon-tui`: `/help`, `/exit`, `/new`,
  `/conv [n]`, `/next`, `/prev`, `/rename`, `/delete`, `/agent`,
  `/chat`, `/model`, `/provider`, `/max-steps`, `/system [text]`,
  `/load <gguf>`, `/history`, `/search <query>` (full-text search
  across all conversations, newest first, with cap), `/tools` /
  `/tools enable|disable <name>` (runtime tool gate; persists),
  `/clear`, `/vault [path|close]`, `/note <path>`, `/find <query>`
  (semantic when an embed model is loaded, FTS5 otherwise),
  `/embed [<gguf>]`.
- **Conversation persistence**: rezon-tui stores all conversations,
  the active conversation id, the active vault path, and disabled
  tools as JSON under
  `<ProjectDirs::config_dir>/conversations.json` (macOS:
  `~/Library/Application Support/com.rezon.rezon-tui/`).
- **rustyline line editor** with persistent history at
  `<config_dir>/history.txt` — arrow keys, Home/End, Ctrl-A/E, Ctrl-W,
  Ctrl-U/K, Ctrl-R reverse-search, ↑/↓ history. Tab completion for
  slash commands and (for path-taking verbs `/load`, `/embed`,
  `/vault`, `/note`) for filesystem paths.
- **Streaming UX**: tokens stream straight to stdout as they arrive.
  Cancellation via Ctrl-C during a stream (flips
  `LlmState`/`AgentRunHandle` cancel flags); second Ctrl-C
  force-exits. Ctrl-D on an empty line exits cleanly. Per-turn stats
  printed in magenta after each response:
  `[ Prompt: N tok | Generation: N tok @ N.N t/s ]`.
- **Agent mode in the TUI**: `--agent` / `/agent`. Tool calls render
  inline as `→ name` then `✓ name: {result}` or `✗ name: error`.
  Tool confirmations prompt inline (`approve tool X with args …
  [y/N] >`) and block the agent loop on a oneshot. `search_notes`
  registers automatically when a vault is open.
- **Agent tool turns persist for replay**: `core::llm::ChatMsg`
  extended with optional `tool_calls` + `tool_call_id` (both
  `#[serde(default)]`, backward-compatible with existing stores).
  After each agent run the spawn block snapshots the full
  `Vec<ChatMessage>` via `UiEvent::AgentHistory` and the REPL
  replaces the conversation's messages with the structured form, so
  subsequent agent runs (including across restarts) see the model's
  prior tool selections + results.
- **Vault + embeddings in the TUI**: opens via `/vault <path>`;
  auto-opened on next launch via the persisted `active_vault`.
  `/find` uses semantic search when an embed model is loaded, FTS5
  otherwise. `/embed <gguf>` loads a separate embedding model;
  catch-up loop indexes new chunks in the background.
- **Spinner** for long-running blocking loads (`/load`, `/embed`,
  startup `--gguf`). Braille frames at 80 ms via a tokio task;
  cursor hidden during spin, restored on stop; suppressed on
  non-tty.
- **Cross-platform color**: `anstyle` for style values, `anstream`
  for output. SGR codes stripped automatically when stdout isn't a
  tty (`rezon-tui --help | cat`); translated to Win32 console API
  calls on legacy Windows; `NO_COLOR` honored.
- **Makefile targets**: `build-tui`, `build-tui-release`,
  `run-tui ARGS="…"`, `run-tui-release ARGS="…"`.
- **Markdown rendering** for assistant responses in chat mode. Tokens
  still stream raw for live feedback; on stream end the REPL counts
  the rows the raw text occupied (terminal-width-aware), scrolls the
  cursor back with `\x1b[<n>A\r\x1b[J`, and re-emits the formatted
  version in place. Hand-rolled renderer (no extra deps) handles
  `**bold**`, `*italic*`, `` `inline code` ``, `#`/`##`/`###`
  headings, `-`/`*`/`1.` lists, `> blockquotes`, and triple-backtick
  fenced code blocks (dimmed, 2-col indent, optional language tag
  noted as `┄ lang`). Skipped when stdout isn't a tty (piped output
  keeps raw markdown) and in agent mode (where inline tool pills
  would be clobbered by the re-render).
- **Embedded fuzzy picker** (`crossterm` + `nucleo-matcher`, no
  alt-screen takeover). Renders below the current cursor, scrolls
  selection into view, cleans up on exit. Bound to:
  - `/conv` — fuzzy pick over conversation titles (the old plain
    listing moved to `/conv list`).
  - `/search [query]` — picker over every non-system / non-tool
    message across all conversations. Optional argument pre-seeds
    the filter. Enter switches to the picked conversation and
    prints the matched message in context.
  - `/tools enable` / `/tools disable` with no name argument —
    picker over the disabled / enabled subset respectively.
  Keys: typing filters, `↑/↓` move, `Enter` selects, `Esc` /
  `Ctrl-C` cancel.
- **Tests** — 58 across the workspace, `make test` green.
  - `rezon-core` (25): vault file ops + path-traversal rejection,
    `vault_resolve_wikilink` modes, list-tree filtering/sorting,
    `chunk_markdown` paragraph splitting + char-offset coverage,
    cloud catalog shape + `resolve_cloud_config` paths,
    `persist/read_last_model` roundtrip, `to_openai_messages`
    tool-role filtering + unknown-role rejection, `ChatMsg` /
    `ChatMessage` serde with optional tool_calls + tool_call_id,
    `ToolRegistry::register`/`get`/`without`/`openai_schemas`.
    `tempfile` added as a dev-dependency.
  - `rezon-tui` (33): markdown renderer (bold / italic edge-case
    guard / inline code / headings / lists / blockquotes / fenced
    code) + `count_rows` wrap-aware row math, `picker::truncate`
    incl. multi-byte chars + max ∈ {0,1}, `Conversation`
    auto-titling, `Store` save/reload roundtrip via on-disk JSON,
    `delete_active` index clamping, `chat_messages_to_msgs`
    role-preserving conversion, `build_agent_messages` replay
    (carries `tool_calls` on assistant turns + `tool_call_id` on
    tool turns; drops orphan tool turns).
- **Per-conversation settings** in `rezon-tui`. Each conversation
  carries optional overrides for `provider`, `model`, `base_url`,
  `api_key`, `agent_mode`, and `show_thinking`; the REPL composes
  effective values per call from `Conversation::settings` falling
  back to CLI defaults. `/model`, `/provider`, `/agent`, `/chat`
  now write to the active conversation (persisted) instead of the
  process. Switching conversations via `/conv` / `/next` / `/prev`
  automatically picks up the new conversation's settings.
- **`/thinking on|off|toggle`** — surface or hide agent reasoning
  blocks (`<think>` tokens emitted by Qwen3 and some Anthropic
  prompt-cache responses). Per-conversation; default off.
  `--show-thinking` CLI flag sets the launch default. Thinking
  deltas stream in dim mode and an inline `C_RESET` precedes the
  next content token so the assistant text isn't tinted.
- **`/history` markdown rendering** — prior assistant turns route
  through `markdown::render` so they show with the same bold /
  italic / heading / list / code-block formatting as live
  responses. System / tool / user lines unchanged.
- **`/clear`** intercepted before `handle_command` so it can call
  `editor.clear_screen()` on the local rustyline editor; the
  editor's internal layout tracking is reset alongside the visible
  viewport, so the next prompt aligns cleanly.
- **`/export <path>` / `/import <path>`** — single-conversation
  round-trip via pretty-printed JSON. Imports get a fresh id (via
  `store::next_id`, promoted to `pub(crate)`) so a re-imported file
  can coexist with the original. Tab completion treats the
  argument as a filesystem path.
- **`/fork`** — duplicate the active conversation. Fresh id, title
  becomes `"<title> (fork)"`, immediately switched to. Persists.
- **`/models [provider]`** — list a provider's recommended models
  (from `crates/rezon-core/models.json`). `*` marks the
  conversation's currently active model; `(default)` marks the
  catalog default. `local` is a special case that reports the
  loaded GGUF path. No-arg uses the active conversation's
  effective provider.
- **Live tokens-per-second in the terminal title** during
  generation. The REPL writes `\x1b]0;rezon · ~N.N tok/s\x07`
  throttled to ~5 Hz; rate is approximated from emitted char
  count / 4 (same heuristic the chat path uses when the provider
  omits usage). `Token` and `Thinking` deltas both feed the
  counter. On `Done` / `Error` the title resets to `rezon`.
  Suppressed when stdout isn't a tty.
- **Workspace hygiene** — `cargo fmt --all` clean and `cargo clippy
  --workspace --all-targets -D warnings` clean. Lints fixed along
  the way:
  - `manual_flatten` in `core::search::vault_search_impl` — switched
    to `for hit in rows.flatten()`.
  - `only_used_in_recursion` in `core::vault::read_tree` — dropped
    the unused `vault: &Path` parameter from the recursive helper.
  - `collapsible_match` on the picker's Up/Down handlers — rewrote
    as match guards (`KeyCode::Up if state.selected > 0 =>`).
  - `items_after_test_module` in `tui::picker` — moved `cleanup`
    above the test module.
  - `missing_transmute_annotations` on
    `core::search::register_sqlite_vec` — `#[allow]` with a comment
    explaining the cast (both sides are `unsafe extern "C" fn`).
  - `too_many_arguments` on `tui::agent::spawn_agent_run` —
    `#[allow]`; collapsing into a struct would only push the
    bag-of-args one layer in.

### Changed
- **Workspace refactor.** Rust code split into a 3-crate Cargo
  workspace under `crates/`:
  - `rezon-core` — provider-agnostic backend: chat (local llama.cpp +
    OpenAI-compatible cloud via `async-openai`), agent loop, tools
    (including `search_notes`), vault file ops, FTS5 + sqlite-vec
    search index, embedding worker + background catch-up loop.
    Zero Tauri references.
  - `rezon-web` — thin Tauri shell wrapping `rezon-core` via
    `TauriChatSink`, `TauriEventSink`, `TauriConfirmationGate`,
    `#[tauri::command]` wrappers, and config-dir resolution. The
    frontend (`src/`) is untouched; all command names and event
    names preserved.
  - `rezon-tui` — terminal REPL described above.
- `src-tauri/` moved to `crates/rezon-web/`. `tauri.conf.json` got
  `frontendDist: "../../dist"`. The `make dev` / `make build`
  targets pass `--config $(TAURI_CONF)` so Tauri finds the
  relocated config; everything else (frontend, dev server, etc.)
  is unchanged.
- `LlmState`, `SearchState`, `EmbedState` are now registered as
  `Arc<T>` in Tauri's managed state so they can be shared between
  the chat command and the agent loop without copies.
- `ChatSink` trait introduced in `rezon-core::llm` to replace the
  previous direct `app.emit("chat-token"|"chat-stats"|"chat-done", …)`
  calls. `TauriChatSink` (in `rezon-web`) preserves the exact event
  names and payloads.
- `SearchState` now takes its data directory at construction
  (`SearchState::new(data_dir)`); the previous `Default` impl
  required an `AppHandle` to resolve `app_data_dir`. `rezon-web`
  builds it inside `.setup()` after the handle is available.
- `EmbedState::load` no longer emits Tauri events directly; the
  shell wrapper (`web::embed::do_load_embed`) emits
  `embed-loading` / `embed-loaded` / `embed-load-error` around the
  core call. `core::embed::ensure_catchup_started` takes
  `Arc<EmbedState>` + `Arc<SearchState>` instead of `AppHandle`.
- `core::llm::to_openai_messages` now skips `tool`-role messages
  (previously errored on unknown role) so a mixed agent/chat
  conversation history flows through the cloud chat endpoint
  cleanly. Same filter on the local chat path.
- `core::agent::tool::ToolContext` lost its
  `app: Option<AppHandle>` field. `agent::tools::search_notes`
  receives `Arc<SearchState>` + `Arc<EmbedState>` at construction
  (`register_search_notes(&mut reg, search, embed)`) instead of
  reaching for them through Tauri state.
- **README** — rewritten for the workspace layout. Now documents
  both shells (`rezon` GUI + `rezon-tui` REPL), describes the
  three-crate split with an annotated file tree, has separate
  Quick-start sections per shell, and lists all `make` targets
  including the new TUI ones. Build/run instructions for
  `rezon-tui` make clear it builds with `cargo` alone (no Bun /
  Tauri prerequisites).
- **`package.json`** — `tauri` npm script now bakes in
  `--config crates/rezon-web/tauri.conf.json` so
  `bun run tauri …` works standalone (no need to go through
  `make dev` / `make build`).
- `SearchState::close_vault(path)` added in core so the TUI's
  `/vault close` actually drops the per-vault index + stops its
  file watcher (the GUI doesn't yet surface this).

### Fixed
- `make dev` / `make build` continue to work after the workspace
  refactor via the `--config $(TAURI_CONF)` flag passed to the
  Tauri CLI.

## [Older entries]

### Changed
- Migrated all headless-component usage from Radix
  (`@radix-ui/react-dialog`, `@radix-ui/react-alert-dialog`,
  `@radix-ui/react-tooltip`) to [Base UI](https://base-ui.com)
  (`@base-ui/react@1.4.1`), the consolidation effort by the same
  authors. Single dep, consistent API. Component name shifts:
  Radix `Overlay` → Base UI `Backdrop`; `Content` → `Popup`; Tooltip
  needs an extra `Positioner` wrapper between `Portal` and `Popup`.
  `Tooltip.Provider` props renamed (`delayDuration` →
  `delay`, `skipDelayDuration` → `timeout`). `Tooltip.Trigger` no
  longer needs `asChild` — it renders a `<button>` itself, so
  `className`/`onClick` go directly on the trigger.
- Provider and Theme native `<select>` elements replaced with Base UI
  `Select` for visual consistency with the rest of the
  themed/Base-UI-styled UI (no more OS-default chevron jumping out
  against the rest of the app). New shared wrapper at `src/Select.tsx`
  used by both the right-sidebar Provider field and the SettingsDrawer
  Theme field; takes `{ value, label }` items and a string
  `onValueChange`.
- Recommended-models field for cloud providers replaced the
  `<select>` + free-text `<input>` pair with a single Base UI
  `Combobox`. Typing filters the list; an explicit "No matches —
  press Enter to use as-is" empty state preserves the free-text
  override path. The Other-provider three-input stack
  (model + base URL + API key) is unchanged.
- Adopted Tailwind CSS v4 via `@tailwindcss/vite`. Component styling
  migrated from hand-rolled CSS classes (`rs-`, `conv-`, `msg-`, etc.)
  to Tailwind utility classes inline in JSX. The CSS-variables theming
  is preserved and surfaced as Tailwind color tokens via
  `@theme inline { --color-bg: var(--bg); ... }`, so utilities like
  `bg-bg` / `text-fg` / `border-border` automatically follow
  `[data-theme="..."]` switches without needing the `dark:` variant.
  `App.css` shrank to roughly: theme variables, markdown-content rules
  (`.md p`, `.md h1`...) that target generated HTML, code-block wrapper,
  and Radix `data-state` keyframes.

### Fixed
- AlertDialog / SettingsDrawer flashed in the top-left quadrant for one
  animation cycle before settling in the center after the Tailwind
  migration. Cause: Tailwind v4 centers via the modern `translate:`
  property (`-translate-x-1/2 -translate-y-1/2`), but the dialog
  pop-in/out keyframes wrote to the legacy `transform: translate(...)`.
  The two properties compose additively, double-translating the dialog
  off-center while the animation ran. Keyframes now animate only
  `scale` + `opacity`, leaving translation entirely to Tailwind.
- Crash on app close inside `__cxa_finalize` →
  `ggml_metal_device_free` →
  `GGML_ASSERT([rsets->data count] == 0) failed`. Cause: the worker
  thread refactor for KV-cache reuse stored only the `mpsc::Sender`
  in `LoadedHandle` and discarded the `JoinHandle`, so on
  `RunEvent::Exit` we'd close the channel and immediately return —
  the worker (and its `LlamaContext`) could still be alive when C++
  static destructors ran on the main thread, leaving live resource
  sets on the metal device. Fix: `LoadedHandle` now stores
  `Option<JoinHandle>` and its `Drop` impl closes the channel and
  joins the worker before returning. `LlmState::shutdown` also flips
  the cancel flag so an in-flight chat aborts immediately rather
  than running to `MAX_NEW_TOKENS`. Same join-on-drop also runs on
  model swap, so the previous model's worker is fully torn down
  before the new one takes over.

### Added
- Multiple conversations with a left sidebar. Conversations have their own
  title, system prompt, message history, and timestamps. Sorted most-recent
  first; rename inline (pencil icon) and delete (trash icon, confirms).
  All persisted to `localStorage` along with the currently selected
  conversation id; conversation titles are auto-derived from the first
  user message.
- Editable per-conversation system prompt in the right sidebar. New
  conversations seed from `settings.defaultSystemPrompt`.
- Settings drawer (Settings button in the sidebar) with theme
  (system/light/dark — applied via `data-theme` on `:root` and CSS
  variables), font size slider (12–20px, applied via root `font-size`),
  and default system prompt for new conversations. Persisted to
  `localStorage`.
- Copy buttons: per-message (revealed on row hover, copies the raw
  content) and per-code-block (revealed on `<pre>` hover, reads
  `pre.innerText` so it strips the highlight markup).
- Token / timing stats per assistant message. Backend emits a
  `chat-stats` event before `chat-done`:
  - Local: exact `promptTokens`, `cachedTokens`, `genTokens`, plus
    `durationMs` measured from the start of `run_chat_with_cache`.
  - Cloud: requests `stream_options.include_usage`; uses the final
    chunk's usage when present, otherwise falls back to a `len/4` char
    estimate so the row is never blank. Also includes `durationMs`.
  Rendered as a compact monospace line under each message:
  `1234 prompt (1100 cached) · 234 gen · 25.0 tok/s · 9.4s · local`.
- Better banner / error states. Load errors render in a dedicated banner
  with a dismiss button; chat errors render as a distinct error-styled
  assistant bubble (monospace, muted-red background) instead of mixing
  with normal markdown content.

### Changed
- Adopted Radix UI primitives for the bits where accessibility matters
  most: `@radix-ui/react-dialog` powers the SettingsDrawer (focus trap,
  ESC-to-close, scroll lock, portal, ARIA), `@radix-ui/react-alert-dialog`
  replaces the native `confirm()` on conversation delete, and
  `@radix-ui/react-tooltip` (wrapped in a top-level `Tooltip.Provider`)
  labels icon-only buttons (sidebar collapse/expand, "+" new chat in the
  collapsed left strip). The primitives are unstyled — CSS lives in
  `App.css` keyed off Radix's `data-state` attributes for fade/pop
  animations. No styling-framework migration: still vanilla CSS + CSS
  variables.
- Three-pane layout: left sidebar (conversations + settings), center
  (chat log + input), right sidebar (provider, model, system prompt).
  Both sidebars are collapsible (chevron toggle, persisted to settings);
  collapsed strips show only an expand button (left also keeps a "+"
  shortcut for new chat). Provider, model row, and the per-conversation
  system prompt textarea moved out of the chat header into a new
  `RightSidebar` component. Provider selection is now a single dropdown
  rather than a radio list.
- Frontend split into modules: `types.ts`, `storage.ts`, `Sidebar.tsx`,
  `RightSidebar.tsx`, `SettingsDrawer.tsx`, `MessageBody.tsx` (extracted
  from `App.tsx`). `App.tsx` is now an orchestrator wiring them together.
- The whole UI is themed via CSS variables (`--bg`, `--fg`, `--accent`,
  `--border`, `--code-bg`, etc.) — no more hard-coded `#4a7dff` /
  `rgba(127,127,127,...)` etc. scattered through `App.css`.

### Added (earlier in this Unreleased)
- KV cache reuse across turns for the local backend. Each loaded model now
  owns a dedicated worker thread that holds the `LlamaContext` and a
  shadow `Vec<LlamaToken>` of tokens currently in the KV cache. On each
  request the worker tokenizes the new full prompt, finds the longest
  common prefix with the cached tokens, calls `clear_kv_cache_seq` to
  truncate the KV cache to that point, and decodes only the divergent
  suffix before sampling. Continuing the same conversation now re-decodes
  ~zero prompt tokens instead of the full history.
- Stop button to abort an in-flight chat. New `cancel_chat` Tauri command
  flips an `AtomicBool` on `LlmState` that the local generation loop and
  the cloud stream loop both poll between iterations. The flag is reset
  at the start of each new `chat`. While streaming, the chat-input Send
  button is replaced by a red Stop button.

### Changed
- `LlmState.loaded` switched from `tokio::sync::Mutex<Option<Loaded>>` to
  `std::sync::Mutex<Option<LoadedHandle>>`. The handle holds the path and
  an `mpsc::Sender<WorkerRequest>` for the worker thread, which owns the
  model and context (`LlamaContext<'a>` is `<'a>`-tied to `LlamaModel` and
  not `Send`, so the only safe way to keep it alive between turns is to
  pin it to a single thread). `model_status` is now a sync command.
- Cloud providers via [`async-openai`](https://github.com/64bit/async-openai)
  0.36 (`chat-completion` feature). Four OpenAI-compatible providers ship
  out of the box. The first three are env-driven with hard-coded base URL,
  default model, and recommended-model list; the last is fully
  user-configurable:
  - **OpenAI** — `OPENAI_API_KEY`, `https://api.openai.com/v1`
  - **Anthropic** — `ANTHROPIC_API_KEY`, `https://api.anthropic.com/v1`
  - **OpenRouter** — `OPENROUTER_API_KEY`, `https://openrouter.ai/api/v1`
  - **Other** — user-configurable: model, base URL, and API key are
    entered in the UI and sent in `chat` opts. Intended for
    OpenAI-compatible servers like Ollama (`http://localhost:11434/v1`),
    LM Studio, or `llama.cpp` `server`. The API key is optional; an empty
    field is sent as `"no-key"` so servers that don't authenticate ignore
    it.
- Provider selector in the UI spans Local + the four cloud providers
  (later changed to a dropdown — see the [Unreleased] entry above).
  Named providers' cloud row shows a recommended-models dropdown alongside
  a free-text override (the text input is canonical and sent to the
  backend). The `other` row instead stacks three free-text inputs: model,
  base URL, API key (password field).
- New Tauri command `cloud_providers()` returns the static provider table
  plus per-provider `apiKeySet` and `userConfigurable`. Replaces
  `openai_available()`.
- `chat` command takes
  `opts: { provider, model?, baseUrl?, apiKey? }` (camelCase). `baseUrl`
  and `apiKey` are only consumed when the provider is user-configurable
  (i.e. `other`). **Breaking**: the previous `openaiModel` /
  `openaiBaseUrl` fields are gone — for the named providers, base URL is
  hard-coded.
- All cloud providers stream deltas through the same `chat-token` /
  `chat-done` events as the local backend.
- Markdown rendering for assistant messages via `react-markdown` with
  `remark-gfm`.
- KaTeX math rendering (`remark-math` + `rehype-katex`), including
  normalization of `\(...\)` and `\[...\]` LaTeX delimiters to `$...$` /
  `$$...$$` so they are picked up by `remark-math`. Skips fenced code blocks.
- Syntax highlighting for code blocks via `rehype-highlight` (highlight.js
  github-dark theme).
- Auto-close of unbalanced fenced code blocks during streaming so partial
  responses render correctly.

### Changed
- `package.json` gained `react-markdown`, `remark-gfm`, `remark-math`,
  `rehype-katex`, `rehype-highlight`, `katex`, `highlight.js`.

## [0.1.0] - initial

### Added
- Tauri 2 + React 19 + Vite scaffold.
- Rust backend in `src-tauri/src/llm.rs` wrapping `llama-cpp-2` 0.1.146 with
  the `metal` feature.
- `LlmState` holding `LlamaBackend` and the currently loaded `LlamaModel`,
  with explicit `shutdown()` on `RunEvent::Exit` to drop the model before
  the backend and avoid ggml-metal static-destructor races.
- Tauri commands: `load_model`, `model_status`, `chat`.
- Frontend events: `model-loading`, `model-loaded`, `model-load-error`,
  `chat-token`, `chat-done`.
- Persistence of the last-loaded model path under the app config dir
  (`last_model.txt`) and auto-load on startup.
- Streaming token generation: prompt is tokenized, decoded once, then
  sampled token-by-token (`temp(0.7)` -> `dist(1234)`) up to `MAX_NEW_TOKENS`
  or EOG, with each piece emitted as a `chat-token` event.
- UTF-8 incremental decoder (`encoding_rs`) so multi-byte tokens aren't
  truncated mid-codepoint.
- Use of model-embedded chat template via `model.chat_template(None)` +
  `apply_chat_template`.
- React UI: model path input, file picker (`@tauri-apps/plugin-dialog`,
  `.gguf` filter), Load button, chat log with role labels, textarea input
  with Enter-to-send / Shift+Enter for newline, auto-scroll on new content.
- `Makefile` with `install`, `dev`, `build`, `web-dev`, `web-build`,
  `check`, `fmt`, `fmt-check`, `lint`, `test`, `clean` targets.
- MIT `LICENSE`.
