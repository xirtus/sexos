#!/usr/bin/env bash
# Inspect a retained guest log; no build, boot or missing-proof success path.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
args=("${OWNERSHIP_LOG:-$ROOT_DIR/.gate_master/serial.log}")
if [[ "${REQUIRE_LAST_RUNNABLE:-0}" == 1 ]]; then args+=(--require-last-runnable); fi
python3 "$ROOT_DIR/scripts/check_scheduler_ownership_log.py" "${args[@]}"
