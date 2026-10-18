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

All dependencies are at latest. `make ci` passes.

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

### `llama-cpp-2`

`=0.1.160`, with `rezon-llama-oai` supplying the OAI-compatible API removed in 0.1.147 (see CHANGELOG). Bumping it:

1. Set the same version in `rezon-llama-oai`, `rezon-core` and `rezon-web` manifests.
2. Diff `llama.cpp/common/chat.h` between the two sys-crate releases; fix `src/shim.cpp`.
3. Check `LlamaModel` is still `#[repr(transparent)]` over its model pointer. `model_ptr` asserts the size only.
4. `make test`, then `run_local_tool_spike.sh` against a tool-aware GGUF. The unit tests cover parsing but not template rendering, which needs a model.

Both workarounds this depends on are described in `docs/dev/workarounds.md`.
