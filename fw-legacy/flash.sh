#!/usr/bin/env bash
set -euo pipefail

CHIP="${CHIP:-PY32F002Ax5}"
TARGET="${TARGET:-thumbv6m-none-eabi}"

args=(
  +nightly flash
  --release
  --target "${TARGET}"
  --chip "${CHIP}"
)

# Enable only when needed (e.g. locked target and reset line wired).
if [[ "${CONNECT_UNDER_RESET:-0}" == "1" ]]; then
  args+=(--connect-under-reset)
fi

cargo "${args[@]}" "$@"
