# llama-cpp-2: expose the raw `llama_model` pointer

Target: [utilityai/llama-cpp-rs](https://github.com/utilityai/llama-cpp-rs), crate `llama-cpp-2`. Status: draft, not filed. Version: 0.1.160.

## Problem

`LlamaModel` has no public accessor for its `*const llama_model`. The field is `pub(crate) model: Ptr<llama_model>` (`src/model.rs:29`). `LlamaContext` has a `model_ptr()`, but it is private (`src/context.rs:77`).

Some FFI functions need the model pointer and have no safe wrapper, e.g. `common_chat_templates_init` in llama.cpp `common/chat.h`. Passing `nullptr` does not work as a fallback. With a null model, llama.cpp skips:

- the `tool_use` template from GGUF metadata;
- BOS/EOS strings and `add_bos` / `add_eos` from the vocab. A template that emits BOS keeps it, and tokenizing with `add_special = true` then adds a second BOS.

## Current workaround

rezon reads the pointer by relying on `LlamaModel` and `Ptr` both being `#[repr(transparent)]` over `NonNull<llama_model>` (`crates/rezon-llama-oai/src/lib.rs`, `model_ptr`). A compile-time size check catches some layout changes, but not all of them. This is why rezon pins `llama-cpp-2` with `=`.

## Proposal

```rust
impl LlamaModel {
    /// The underlying `llama_model`. Valid for the lifetime of `self`.
    #[must_use]
    pub fn as_ptr(&self) -> *const llama_cpp_sys_2::llama_model {
        self.model.as_ptr()
    }
}
```

Returning `*const` matches `Ptr::as_ptr` and the crate's const rules. Dereferencing the pointer still needs `unsafe`, so the accessor adds no new unsafety.

## Alternatives

- `unsafe fn as_ptr`. Not needed: obtaining a raw pointer is safe. Only dereferencing it is unsafe.
- Wrap `common_chat_templates_init` and friends in llama-cpp-2. This brings back what 0.1.147 removed; see [llama-cpp-sys-2-export-source-path.md](llama-cpp-sys-2-export-source-path.md).

## Downstream cleanup once fixed

Replace the body of `model_ptr` with `model.as_ptr()` and drop the layout assertion. The `=` pin stays, because `common/chat.h` still changes between releases.
