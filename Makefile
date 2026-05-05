CARGO_MANIFEST := src-tauri/Cargo.toml

.PHONY: help install dev build web-dev web-build check fmt fmt-check lint test clean

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  %-14s %s\n", $$1, $$2}'

install: ## Install JS deps
	bun install

dev: ## Run Tauri app in dev mode
	bun run tauri dev

build: ## Build Tauri app for release
	bun run tauri build

web-dev: ## Run Vite dev server only (no Tauri)
	bun run dev

web-build: ## Build frontend only
	bun run build

check: ## cargo check
	cargo check --manifest-path $(CARGO_MANIFEST)

fmt: ## Format Rust code
	cargo fmt --manifest-path $(CARGO_MANIFEST)

fmt-check: ## Verify Rust formatting
	cargo fmt --manifest-path $(CARGO_MANIFEST) -- --check

lint: ## Clippy with warnings as errors
	cargo clippy --manifest-path $(CARGO_MANIFEST) --all-targets -- -D warnings

test: ## Run Rust tests
	cargo test --manifest-path $(CARGO_MANIFEST)

clean: ## Remove build artifacts
	rm -rf node_modules dist src-tauri/target
