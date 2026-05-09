#!/usr/bin/env bash
# Run the local-model tool-calling spike.
#
# Usage:
#   run_local_tool_spike.sh <model.gguf> ["prompt"]
#
# Recommended GGUF models (tool-aware chat template required):
#   Llama 3.1/3.2/3.3 Instruct, Qwen 2.5/3 Instruct (>=7B), Mistral Nemo
#   Instruct, Hermes 3.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <model.gguf> [prompt]" >&2
  exit 1
fi

MODEL_PATH="$1"
PROMPT="${2:-What is 17 * 23? Use the calculator tool.}"

cd "$(dirname "$0")/.."
cargo run --quiet --example local_tool_spike -- "$MODEL_PATH" "$PROMPT"
