# TODO

Known gaps and ideas, roughly ordered by usefulness. Not committed scope.

## Inference / backend

- [ ] Cancel an in-flight `chat` (currently runs to `MAX_NEW_TOKENS` or EOG;
      no stop signal from the UI).
- [ ] Reuse the `LlamaContext` across turns and keep the KV cache instead of
      rebuilding context + re-decoding the full prompt on every call.
- [ ] Configurable sampler: temperature, top-p, top-k, repeat penalty, seed
      (currently hard-coded `temp(0.7)` + `dist(1234)`).
- [ ] Configurable `n_ctx`, `max_new_tokens`, `n_gpu_layers` (currently
      hard-coded constants in `llm.rs`).
- [ ] Surface load progress / mmap status; `model-loading` only carries the
      path.
- [ ] Unload / swap model without restarting the app.
- [ ] Graceful error when the model has no embedded chat-template metadata
      (today it just bubbles up as a string).
- [ ] Handle prompts that overflow `n_ctx` (truncate history vs. error).
- [ ] Non-macOS build paths (CUDA / CPU-only feature flags).

## Frontend / UX

- [ ] Persist conversation history across restarts.
- [ ] Multiple conversations / sidebar.
- [ ] Editable system prompt (currently a const in `App.tsx`).
- [ ] Stop button while streaming.
- [ ] Copy-message and copy-code-block buttons.
- [ ] Token / timing stats (tok/s, prompt vs. gen tokens).
- [ ] Better empty / error states; the current load-error banner is plain
      text.
- [ ] Theme: light mode, font-size control.

## Engineering

- [ ] Actual Rust tests (`make test` runs `cargo test` but `llm.rs` has none).
- [ ] Frontend tests / typecheck in CI.
- [ ] CI workflow (fmt-check, clippy, cargo test, vite build).
- [ ] Replace `eprintln!` with structured logging (`tracing`).
- [ ] Audit `unwrap()` on the std `Mutex` in `LlmState` for poisoning.
- [ ] Consider a typed event payload for `chat-token` instead of bare string
      (e.g. include a turn id so late events can't bleed into a new turn).
