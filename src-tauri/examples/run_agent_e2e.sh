#!/usr/bin/env bash
# Run the phase-2 agent loop end-to-end against OpenRouter.
#
# Requires OPENROUTER_API_KEY in the environment.
# Override the model with OPENROUTER_MODEL.

set -euo pipefail

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  echo "OPENROUTER_API_KEY is not set in this shell." >&2
  exit 1
fi

PROMPT="${1:-What time is it? Use the current_time tool.}"

cd "$(dirname "$0")/.."
cargo run --quiet --example agent_e2e -- "$PROMPT"
