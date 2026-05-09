#!/usr/bin/env bash
# Run the ReAct agent prototype against OpenRouter using a Claude Sonnet model.
#
# Requires OPENROUTER_API_KEY in the environment.
# Pass a prompt as $1, or use the default.

set -euo pipefail

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  echo "OPENROUTER_API_KEY is not set in this shell." >&2
  exit 1
fi

PROMPT="${1:-What is 17 * 23, then add the current hour to it?}"
MODEL="${OPENROUTER_MODEL:-anthropic/claude-sonnet-4}"

cd "$(dirname "$0")/.."

OPENAI_API_KEY="$OPENROUTER_API_KEY" \
OPENAI_BASE_URL="https://openrouter.ai/api/v1" \
OPENAI_MODEL="$MODEL" \
cargo run --quiet --example react_agent -- "$PROMPT"
