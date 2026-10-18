# llama-cpp-sys-2: export the llama.cpp source path to dependent crates

Target: [utilityai/llama-cpp-rs](https://github.com/utilityai/llama-cpp-rs), crate `llama-cpp-sys-2`. Status: draft, not filed. Version: 0.1.160.

## Problem

A dependent crate cannot compile C++ against `llama.cpp/common/*.h`. The sys crate builds and links `llama-common` under the `common` feature, but never tells dependents where the headers are.

`build.rs` declares `links = "llama"` and emits two metadata keys:

- `ggml_cmake_dir` (`<OUT_DIR>/lib/cmake`)
- `backends_dir` (only with `dynamic-backends`)

The source tree is `Path::new(&manifest_dir).join("llama.cpp")` (`build.rs:483`). It is not exported.

## Use case

0.1.147 removed `llama_cpp_2::openai` and the `wrapper_oai.cpp` shim. Downstream projects that need OAI-compatible templating and tool-call parsing must now write their own shim over `common/chat.h`. The headers have to match the `llama-common` the sys crate linked, so a vendored copy is not safe.

rezon works around this by deriving `<OUT_DIR>` from `DEP_LLAMA_GGML_CMAKE_DIR` and reading `CMAKE_HOME_DIRECTORY` from `<OUT_DIR>/build/CMakeCache.txt` (`crates/rezon-llama-oai/build.rs`). This breaks if the CMake build directory layout changes.

## Proposal

One line in `build.rs`:

```rust
println!("cargo:root={}", llama_src.display());
```

Dependents then read `DEP_LLAMA_ROOT` and add `common/`, `include/` and `ggml/include/` to their include path.

## Alternatives

- Export `cargo:include=` with a list of directories. This is more specific, but Cargo metadata values are single strings, so the list needs a separator.
- Restore the OAI-compatible wrapper. This removes the need entirely, but it puts `common/chat` churn back on the maintainers. They may have dropped the wrapper for that reason (inference: the release notes for 0.1.147 were not checked).

## Downstream cleanup once fixed

Remove `llama_src()`'s CMake-cache branch in `crates/rezon-llama-oai/build.rs`, and the TODO.md item.
