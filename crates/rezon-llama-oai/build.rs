//! Compiles `src/shim.cpp` against the llama.cpp tree that llama-cpp-sys-2
//! builds, so the headers match the `llama-common` library it links.
//!
//! The sys crate exports no source path, only `DEP_LLAMA_GGML_CMAKE_DIR`
//! (`<OUT_DIR>/lib/cmake`). Its CMake cache records the source directory.
//! `REZON_LLAMA_SRC` overrides the lookup.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn llama_src() -> PathBuf {
    println!("cargo:rerun-if-env-changed=REZON_LLAMA_SRC");
    if let Ok(dir) = env::var("REZON_LLAMA_SRC") {
        return PathBuf::from(dir);
    }
    let cmake_dir = env::var("DEP_LLAMA_GGML_CMAKE_DIR")
        .expect("llama-cpp-sys-2 did not set DEP_LLAMA_GGML_CMAKE_DIR; set REZON_LLAMA_SRC");
    let sys_out = Path::new(&cmake_dir)
        .ancestors()
        .nth(2)
        .expect("DEP_LLAMA_GGML_CMAKE_DIR has fewer than two parents");
    let cache_path = sys_out.join("build").join("CMakeCache.txt");
    let cache = fs::read_to_string(&cache_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", cache_path.display()));
    cache
        .lines()
        .find_map(|l| l.strip_prefix("CMAKE_HOME_DIRECTORY:INTERNAL="))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("no CMAKE_HOME_DIRECTORY in {}", cache_path.display()))
}

fn main() {
    let src = llama_src();
    assert!(
        src.join("common/chat.h").is_file(),
        "{} is not a llama.cpp tree",
        src.display()
    );
    println!("cargo:rerun-if-changed=src/shim.cpp");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("src/shim.cpp")
        .include(src.join("common"))
        .include(src.join("include"))
        .include(src.join("ggml/include"))
        .include(src.join("vendor"))
        // Upstream headers define unused static helpers.
        .flag_if_supported("-Wno-unused-function");
    // llama-cpp-sys-2 already links the C++ runtime on Apple; a second
    // `-lc++` is redundant.
    if env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        build.cpp_link_stdlib(None);
    }
    build.compile("rezon_llama_oai_shim");
}
