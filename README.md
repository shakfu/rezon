# rezon

LLM chat desktop app. Tauri 2 + React 19 frontend, Rust backend with
interchangeable providers:

- **Local**: [`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) with Metal
  acceleration, loading any `.gguf` model from disk.
- **Cloud (OpenAI / Anthropic / OpenRouter)**:
  [`async-openai`](https://github.com/64bit/async-openai) client pointed at
  each provider's OpenAI-compatible endpoint.
- **Other**: same `async-openai` client with model + base URL + API key
  entered in the UI. Targets any OpenAI-compatible server — Ollama,
  LM Studio, `llama.cpp` `server`, self-hosted gateways, etc.

The provider is chosen per-message via a dropdown in the right sidebar.
Named cloud providers expose a recommended-models dropdown alongside a
free-text model override; **Other** instead takes model + base URL + API
key as free text.

### Cloud providers

| key          | env var              | base URL                             |
| ------------ | -------------------- | ------------------------------------ |
| `openai`     | `OPENAI_API_KEY`     | `https://api.openai.com/v1`          |
| `anthropic`  | `ANTHROPIC_API_KEY`  | `https://api.anthropic.com/v1`       |
| `openrouter` | `OPENROUTER_API_KEY` | `https://openrouter.ai/api/v1`       |
| `other`      | (entered in UI)      | (entered in UI)                      |

The recommended-models lists for the named providers live in
`src-tauri/src/llm.rs`.

## Status

Working end-to-end on macOS:

- Load a `.gguf` model from disk via path input or file picker.
- Last-loaded model path is persisted to the app config dir and auto-loaded
  on startup.
- Streaming chat: tokens are emitted from Rust via Tauri events
  (`chat-token` / `chat-done`) and rendered incrementally in the UI.
- Assistant messages render as Markdown (GFM), with KaTeX math (`$...$`,
  `$$...$$`, plus `\(...\)` / `\[...\]` normalized to dollar delimiters) and
  syntax-highlighted code blocks (highlight.js, github-dark).
- Model and backend are torn down cleanly on app exit. `LoadedHandle::Drop`
  closes the worker channel and joins the thread before returning; only
  then is the `Arc<LlamaBackend>` released. This guarantees the
  `LlamaContext` and its KV-cache buffers are gone before C++ static
  destructors run on the metal device — the alternative is the
  `GGML_ASSERT([rsets->data count] == 0)` crash inside `__cxa_finalize`.
- Each loaded local model gets a dedicated worker thread that holds the
  `LlamaContext` and reuses the KV cache across turns: only the divergent
  suffix of each new prompt is decoded.
- Three-pane layout: left sidebar (conversations + Settings), center
  (chat), right sidebar (provider + model + per-conversation system
  prompt). Both sidebars collapse to a 40px strip; collapsed state is
  persisted.
- Multiple conversations with per-conversation system prompt,
  copy-message / copy-code-block buttons, per-message token + timing
  stats, and a settings drawer (theme, font size, default system prompt).
  All state is persisted to `localStorage`.

## Architecture

```
src/                    React + Vite frontend
  App.tsx               Orchestrator: state, event listeners, layout
  App.css               Tailwind v4 entry + theme variables + markdown/code CSS
  Sidebar.tsx           Left: conversations list + new/rename/delete + settings
  RightSidebar.tsx      Right: provider, model, per-conversation system prompt
  SettingsDrawer.tsx    Theme, font size, default system prompt (Base UI Dialog)
  MessageBody.tsx       Markdown + math + code highlighting + copy buttons
  storage.ts            localStorage wrappers for conversations + settings
  types.ts              Conversation, Msg, Settings, MsgStats, ...
src-tauri/src/
  main.rs               Tauri entry point
  lib.rs                Builder, command registration, auto-load on setup
  llm.rs                Worker thread, providers, chat / cancel commands
```

Tauri commands exposed to the frontend:

- `load_model(path)` -> `ModelStatus`
- `model_status()` -> `ModelStatus`
- `cancel_chat()` -> `()` (signals the in-flight `chat` to abort)
- `cloud_providers()` -> `CloudProviderInfo[]` (key, label, envVar,
  defaultModel, recommendedModels, apiKeySet, userConfigurable)
- `chat(messages, opts)` -> `String` (streams via `chat-token` events).
  `opts = { provider: "local" | "openai" | "anthropic" | "openrouter" |
  "other", model?, baseUrl?, apiKey? }`. `baseUrl` and `apiKey` are
  consumed only by `other`; named providers use their hard-coded base URL
  and read their API key from the corresponding env var.

Events emitted to the frontend:

- `model-loading`, `model-loaded`, `model-load-error`
- `chat-token` (delta string), `chat-stats` (`{ provider, promptTokens?,
  cachedTokens?, genTokens, durationMs }`), `chat-done` (full string)

## Defaults

Hard-coded in `src-tauri/src/llm.rs`:

- `N_CTX = 4096`
- `MAX_NEW_TOKENS = 1024`
- `N_GPU_LAYERS = 999` (offload everything; Metal feature enabled)
- Sampler chain: `temp(0.7)` then `dist(1234)` (fixed seed)

The default system prompt for new conversations lives in `DEFAULT_SETTINGS`
(`src/types.ts`) and can be overridden per-conversation in the right
sidebar or globally in the Settings drawer.

The model's own chat template (`model.chat_template(None)`) is used to format
the prompt; models without embedded chat-template metadata will fail to chat.

## Requirements

- macOS with Metal (other platforms untested).
- Rust toolchain, Bun, Tauri 2 prerequisites.
- For the local backend: a GGUF model file with chat-template metadata.
- For named cloud backends: the relevant API key set in the environment
  when launching the app — `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, or
  `OPENROUTER_API_KEY`. Recommended models and base URLs are baked into
  `src-tauri/src/llm.rs`; free-text model override is available from the UI.
- For `other`: nothing in the environment. Model, base URL, and (optional)
  API key are entered in the UI per session.

## Develop

```
make install      # bun install
make dev          # bun run tauri dev
make web-dev      # vite only, no Tauri shell
make build        # release build
make check        # cargo check
make fmt          # cargo fmt
make lint         # clippy -D warnings
make test         # cargo test
```

See `CHANGELOG.md` for history and `TODO.md` for known gaps.
