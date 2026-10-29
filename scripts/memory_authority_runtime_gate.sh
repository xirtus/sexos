#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 "$ROOT_DIR/scripts/check_memory_authority_log.py" "${MEMORY_AUTHORITY_LOG:-$ROOT_DIR/.gate_master/serial.log}"
