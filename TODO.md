# TODO

Known gaps and ideas, roughly ordered by usefulness. Not committed scope.

## Inference / backend

- [ ] Cloud: forward sampling params (temperature, top_p, max_tokens) from
      the UI instead of using API defaults.
- [ ] Cloud: surface non-stream errors with HTTP status / body, not just
      `format!("{e}")`.
- [ ] Persist provider choice + per-provider chosen model across restarts.
- [ ] User-overridable cloud-models config at
      `~/Library/Application Support/<app>/cloud-models.json`, replacing
      the static lists in `llm.rs` when present.
- [ ] Save the API key entered for the `other` provider (currently it
      lives only in React state and is lost on app restart). Likely
      destination: OS keychain.
- [ ] Re-check API-key env vars per request rather than only at app
      launch, so users don't have to relaunch after exporting a key
      (`cloud_providers()` only snapshots availability at call time).
- [ ] Load API keys from a `.env` file via
      [`dotenvy`](https://crates.io/crates/dotenvy). For `tauri dev`,
      call `dotenvy::dotenv().ok()` early in `lib.rs::run()`. For
      packaged builds, resolve a path under the app config dir
      (`dotenvy::from_path(...)`) since the cwd isn't predictable.
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
      Now that the KV cache persists across turns, a sliding-window
      eviction (drop oldest non-system tokens via `clear_kv_cache_seq` +
      `kv_cache_seq_add`) is more useful than just erroring.
- [ ] Non-macOS build paths (CUDA / CPU-only feature flags).

## Frontend / UX

- [ ] Persist conversation history across restarts.
- [ ] Multiple conversations / sidebar.
- [ ] Editable system prompt (currently a const in `App.tsx`).
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
