# Workarounds

Code that depends on upstream internals. Each entry names its trigger, its failure modes, and the upstream fix that removes it.

## `rezon-llama-oai`

Both entries depend on the exact llama-cpp-2 / llama-cpp-sys-2 release (0.1.160). The `=` pins are the real safeguard; the bump checklist is in `UPGRADING.md`.

### Locating the llama.cpp headers

`crates/rezon-llama-oai/build.rs`, `llama_src()`.

**Why.** `src/shim.cpp` must compile against the same `common/chat.h` that llama-cpp-sys-2 compiled into `llama-common`. The sys crate does not export its source path.

**How.**

1. If `REZON_LLAMA_SRC` is set, use it.
2. Otherwise read `DEP_LLAMA_GGML_CMAKE_DIR` (`<OUT_DIR>/lib/cmake`), which the sys crate does export.
3. Go up two directories to the sys crate's `<OUT_DIR>`.
4. Read `CMAKE_HOME_DIRECTORY:INTERNAL=` from `<OUT_DIR>/build/CMakeCache.txt`.
5. Panic unless `common/chat.h` exists there.

**Breaks if:**

- the sys crate stops emitting `ggml_cmake_dir`;
- it moves its CMake build directory, or the `cmake` crate changes its default;
- the build uses `system-ggml`, which may not install the ggml cmake package (unverified).

All of these fail at build time. Set `REZON_LLAMA_SRC` to get past them.

**Removed by:** [issues/llama-cpp-sys-2-export-source-path.md](issues/llama-cpp-sys-2-export-source-path.md).

### Reading the raw model pointer

`crates/rezon-llama-oai/src/lib.rs`, `model_ptr()`.

**Why.** `common_chat_templates_init` needs the `llama_model*`, which llama-cpp-2 keeps `pub(crate)`. A null model does not work. llama.cpp then skips:

- the GGUF `tool_use` template;
- the vocab's BOS/EOS strings and `add_bos` / `add_eos`. A template that emits BOS keeps it, and tokenizing adds a second one.

**How.** `LlamaModel` is `#[repr(transparent)]` over `Ptr<llama_model>`, which is `#[repr(transparent)]` over `NonNull<llama_model>`. The code reads `&LlamaModel` as `*const NonNull<llama_model>`. A `const` assertion checks the sizes match.

**Breaks if:**

- `repr(transparent)` is dropped or a second field is added. The size check catches this only if the size changes. A zero-sized field keeps the size the same, and `repr(transparent)` would no longer guarantee the layout.
- the field becomes a different type of the same size. The size check misses this. It fails at runtime, most likely as a crash.

This is the riskier entry. Step 3 of the `UPGRADING.md` checklist is a manual check for it.

**Removed by:** [issues/llama-cpp-2-expose-raw-model-pointer.md](issues/llama-cpp-2-expose-raw-model-pointer.md).

### If upstream declines

Vendor or fork llama-cpp-sys-2. We would then own the source path and the bindings, and neither workaround is needed. The cost is tracking upstream llama.cpp ourselves.
