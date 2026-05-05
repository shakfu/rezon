# Changelog

All notable changes to this project. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed
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
