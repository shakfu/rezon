# Changelog

All notable changes to this project. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
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
