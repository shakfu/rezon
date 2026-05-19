CARGO_MANIFEST := Cargo.toml
TAURI_CONF := crates/rezon-web/tauri.conf.json

.PHONY: help install dev build build-tui build-tui-release run-tui run-tui-release \
		web-dev web-build check fmt fmt-check lint test clean

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  %-14s %s\n", $$1, $$2}'

install: ## Install JS deps
	@bun install

dev: ## Run Tauri app in dev mode
	@bun run tauri dev --config $(TAURI_CONF)

build: ## Build Tauri app for release
	@bun run tauri build --config $(TAURI_CONF)

build-tui: ## build tui (debug)
	@cargo build -p rezon-tui

build-tui-release: ## build tui (release)
	@cargo build -p rezon-tui --release

run-tui: ## run tui (debug). Pass args via ARGS="..."
	@cargo run -p rezon-tui -- $(ARGS)

run-tui-release: ## run tui (release). Pass args via ARGS="..."
	@cargo run -p rezon-tui --release -- $(ARGS)

web-dev: ## Run Vite dev server only (no Tauri)
	@bun run dev

web-build: ## Build frontend only
	@bun run build

check: ## cargo check (workspace)
	@cargo check --workspace

fmt: ## Format Rust code (workspace)
	@cargo fmt --all

fmt-check: ## Verify Rust formatting (workspace)
	@cargo fmt --all -- --check

lint: ## Clippy with warnings as errors (workspace)
	@cargo clippy --workspace --all-targets -- -D warnings

test: ## Run Rust tests + clippy (workspace, warnings = errors)
	@cargo test --workspace
	@cargo clippy --workspace --all-targets -- -D warnings

clean: ## Remove build artifacts
	@rm -rf node_modules dist target crates/rezon-web/target
