# Upgrading Dependencies

This document records the state of dependency upgrades: what was applied safely, what remains available as a major (breaking) upgrade, and the concrete issues encountered along the way. It is meant to save the next person the rediscovery work.

Last reviewed: 2026-07-04.

## Project layout

Two dependency ecosystems live side by side:

- **Rust / Cargo** — workspace of three crates (`rezon-core`, `rezon-tui`, `rezon-web`). Manifests in `Cargo.toml` + each `crates/*/Cargo.toml`; resolved versions in `Cargo.lock`.

- **JS / Bun** — frontend (React + Vite + Tauri). Manifest in `package.json`; resolved versions in `bun.lock`.

Verify any upgrade with:

```
make test        # cargo test --workspace + clippy -D warnings
bun run build    # tsc + vite build
```

## Applied (safe, within existing semver ranges)

These were applied via `cargo update` and `bun update`. Only lockfiles changed; no manifest ranges were edited. All tests, clippy, and the frontend build pass.

### Rust (`cargo update`)

~80 crates advanced to their latest semver-compatible versions, including:

- `tauri` 2.11.2 -> 2.11.5 (and the `tauri-*` family)

- `reqwest` 0.13.3 -> 0.13.4

- `rustls` 0.23.40 -> 0.23.41

- `anyhow` 1.0.102 -> 1.0.103

- `serde_json` 1.0.149 -> 1.0.150

- plus the usual transitive churn (tokio deps, `time`, `uuid`, `regex`, etc.)

One crate was **deliberately held back** — see the `llama-cpp-2` issue below.

### JS (`bun update`)

- `react` / `react-dom` 19.2.5 -> 19.2.7

- `@milkdown/*` (all packages) 7.21.1 -> 7.21.2

- `tailwindcss` + `@tailwindcss/vite` 4.2.4 -> 4.3.2

- `vite` 7.3.2 -> 7.3.6

- `@base-ui/react` 1.4.1 -> 1.6.0

- `katex` 0.16.45 -> 0.16.47

- `@tauri-apps/api` 2.11.0 -> 2.11.1, `@tauri-apps/cli` 2.11.0 -> 2.11.4

- `@types/react` 19.2.14 -> 19.2.17

## Issues encountered

### `llama-cpp-2` 0.1.146 -> 0.1.150 breaks the build (held back)

`cargo update` pulled `llama-cpp-2` from 0.1.146 to 0.1.150. Because the crate is pre-1.0, Cargo treats a `0.1.x` minor bump as semver-compatible and applies it automatically — but the crate shipped **breaking API changes** in that range. The build failed in `crates/rezon-core/src/llm.rs` with:

- `error[E0432]: unresolved import llama_cpp_2::openai` — the `openai` module was removed/relocated.

- `error[E0599]: no method named apply_chat_template_oaicompat found for &LlamaModel` — that method was removed/renamed.

- Several downstream `E0277` "the size for values of type str cannot be known at compile-time" errors, cascading from the two changes above.

**Resolution:** pinned back to the known-good version:

```
cargo update -p llama-cpp-2 --precise 0.1.146
cargo update -p llama-cpp-sys-2 --precise 0.1.146
```

To actually adopt 0.1.150+ later, port `crates/rezon-core/src/llm.rs` to the new API (replace the `llama_cpp_2::openai` usage and the `apply_chat_template_oaicompat` call with their current equivalents). Consider pinning `llama-cpp-2` to a tighter range (e.g. `=0.1.146`) in `crates/rezon-core/Cargo.toml` and `crates/rezon-web/Cargo.toml` to stop future `cargo update` runs from silently re-breaking the build.

## Available major upgrades (out-of-range, breaking — not applied)

These require editing version ranges in `package.json` and likely code/config changes. They were left for an explicit decision.

| Package               | Current | Latest | Notes                                          |
|-----------------------|---------|--------|------------------------------------------------|
| vite                  | 7.3.6   | 8.1.3  | Major; plugin/config compatibility.            |
| @vitejs/plugin-react  | 4.7.0   | 6.0.3  | Major; tied to the vite 8 upgrade.             |
| typescript            | 5.8.3   | 6.0.3  | Major; may introduce new type errors.          |
| katex                 | 0.16.47 | 0.17.0 | Pre-1.0 minor, but may change rendering output.|
| llama-cpp-2 (Rust)    | 0.1.146 | 0.1.150| Breaks the build — see issue above.            |

Recommended batching: `vite` 8 + `@vitejs/plugin-react` 6 + `typescript` 6 should be upgraded together as one unit, then verified with `bun run build`. `katex` and `llama-cpp-2` are independent and can each be done on their own.
