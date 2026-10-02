# Upgrading Dependencies

This document records the state of dependency upgrades: what was applied safely, what remains available as a major (breaking) upgrade, and the concrete issues encountered along the way. It is meant to save the next person the rediscovery work.

Last reviewed: 2026-10-02.

## Project layout

Two dependency ecosystems live side by side:

- **Rust / Cargo** — workspace of three crates (`rezon-core`, `rezon-tui`, `rezon-web`). Manifests in `Cargo.toml` + each `crates/*/Cargo.toml`; resolved versions in `Cargo.lock`.

- **JS / Bun** — frontend (React + Vite + Tauri). Manifest in `package.json`; resolved versions in `bun.lock`.

Verify any upgrade with:

```text
make test        # cargo test --workspace + clippy -D warnings
bun run build    # tsc + vite build
```

## State as of 2026-10-02

All dependencies are at latest except `llama-cpp-2`. `make ci` passes.

### Majors applied

| Package | From | To | Code change |
|-|-|-|-|
| `thiserror` | 1 | 2 | none |
| `sha2` | 0.10 | 0.11 | Digest no longer implements `LowerHex`; `key_fingerprint` hex-encodes bytes. Output unchanged. |
| `directories` | 5 | 6 | none; resolved paths unchanged (only a Windows FFI type in `dirs-sys` differs) |
| `anstream` | 0.6 | 1 | none |
| `pulldown-cmark` | 0.12 | 0.13 | none |
| `crossterm` | 0.28 | 0.29 | none |
| `rustyline` | 14 | 18 | none |
| `notify` | 6 | 8 | none |
| `keyring` | 3 | 4 | Manifest only: default `v1` feature replaces the per-platform flags. Entries are addressed as before on all three platforms, so v3-saved keys stay readable. Linux drops the keyutils cache layer. |
| `rusqlite` | 0.32 | 0.40 | none; `sqlite_vec_registers_and_answers_a_knn_query` covers the extension transmute |
| `async-openai` | 0.36 | 0.42 | none; changes are new optional fields and a stream wrapper type |
| `vite` + `@vitejs/plugin-react` | 7, 4 | 8, 6 | none |
| `typescript` | 5.8 | 7 | none |
| `katex` | 0.16 | 0.19 | `overrides: {katex: "$katex"}` in `package.json`. `rehype-katex` 7.0.1 (latest) declares `^0.16.0`; without the override its renderer stays 0.16 while the app loads 0.19 CSS. It calls only `renderToString`, which 0.17-0.19 did not change. The 0.18 class prefixing needs no app change: no app CSS targets KaTeX classes. |

### Held back

- **`llama-cpp-2` / `llama-cpp-sys-2`** at `=0.1.146`. 0.1.147 removed `llama_cpp_2::openai` and `apply_chat_template_oaicompat`, which `llm.rs` uses for OAI-compatible templating and tool-call parsing. 0.1.158 offers only `llama_chat_apply_template` (built-in templates, no Jinja, no tools) plus lazy grammar samplers. The tokenizer methods moved to `model.vocab()`. See the comment in `crates/rezon-core/Cargo.toml`.

  Porting options:
  1. C++ shim over llama.cpp `common/chat.cpp`, which the sys crate still compiles under the `common` feature. The sys crate does not export the llama.cpp source path, and `common/chat` changes often.
  2. Rust: `minijinja` for GGUF templates, the lazy grammar samplers for constrained tool calls, and a parser per model family.
  3. Run `llama-server` as a child process and reuse the `async-openai` path. Removes `llama-cpp-2` from chat, but changes packaging.
