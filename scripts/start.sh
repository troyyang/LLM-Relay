#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

CONFIG_PATH="${LLM_RELAY_CONFIG:-${PROJECT_ROOT}/config/config.yaml}"
BIN_PATH="${LLM_RELAY_BIN:-${PROJECT_ROOT}/target/release/llm-relay}"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<USAGE
Usage: scripts/start.sh [config-path]

Environment:
  LLM_RELAY_CONFIG  Config file path. Defaults to config/config.yaml.
  LLM_RELAY_BIN     Binary path. Defaults to target/release/llm-relay.

If the release binary exists, this script runs it directly.
Otherwise it falls back to: cargo run -- --config <config-path>
USAGE
  exit 0
fi

if [[ $# -gt 0 ]]; then
  CONFIG_PATH="$1"
fi

if [[ ! -f "${CONFIG_PATH}" ]]; then
  echo "config file not found: ${CONFIG_PATH}" >&2
  exit 1
fi

cd "${PROJECT_ROOT}"

if [[ -x "${BIN_PATH}" ]]; then
  exec "${BIN_PATH}" --config "${CONFIG_PATH}"
fi

exec cargo run -- --config "${CONFIG_PATH}"
