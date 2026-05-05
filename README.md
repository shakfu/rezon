# rezo

Local LLM chat desktop app. Tauri 2 + React 19 frontend, Rust backend wrapping
[`llama-cpp-2`](https://crates.io/crates/llama-cpp-2) with Metal acceleration.

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
- Model and backend are torn down cleanly on app exit to avoid races with
  ggml-metal C++ static destructors.

## Architecture

```
src/                    React + Vite frontend
  App.tsx               Chat UI, model loader, event listeners, markdown render
  App.css               Styles
src-tauri/src/
  main.rs               Tauri entry point
  lib.rs                Builder, command registration, auto-load on setup
  llm.rs                LlamaBackend / LlamaModel lifecycle, chat command
```

Tauri commands exposed to the frontend:

- `load_model(path)` -> `ModelStatus`
- `model_status()` -> `ModelStatus`
- `chat(messages)` -> `String` (also streams via `chat-token` events)

Events emitted to the frontend:

- `model-loading`, `model-loaded`, `model-load-error`
- `chat-token`, `chat-done`

## Defaults

Hard-coded in `src-tauri/src/llm.rs`:

- `N_CTX = 4096`
- `MAX_NEW_TOKENS = 1024`
- `N_GPU_LAYERS = 999` (offload everything; Metal feature enabled)
- Sampler chain: `temp(0.7)` then `dist(1234)` (fixed seed)
- System prompt (frontend, `App.tsx`): "You are a concise, helpful assistant."

The model's own chat template (`model.chat_template(None)`) is used to format
the prompt; models without embedded chat-template metadata will fail to chat.

## Requirements

- macOS with Metal (other platforms untested).
- Rust toolchain, Bun, Tauri 2 prerequisites.
- A GGUF model file with chat-template metadata.

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
