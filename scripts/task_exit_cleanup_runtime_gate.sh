#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
args=("${PAGING_LOG:-$ROOT_DIR/.gate_master/serial.log}")
if [[ "${REQUIRE_PAGING_QUOTA:-0}" == 1 ]]; then args+=(--quota); fi
python3 "$ROOT_DIR/scripts/check_task_exit_cleanup_log.py" "${args[@]}"
