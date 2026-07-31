#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

PROFILE_ARGS=()
if [[ "${1:-}" == "--release" ]]; then
  PROFILE_ARGS+=(--release)
  shift
fi

export PATH="$HOME/.cargo/bin:$PATH"
cargo run \
  "${PROFILE_ARGS[@]}" \
  --manifest-path "${PROJECT_DIR}/src-tauri/Cargo.toml" \
  --bin bench \
  -- "$@"
