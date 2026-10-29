#!/usr/bin/env bash
# Inspect retained evidence; never build or synthesize missing observations.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${REQUIRE_PAGING_QUOTA:-0}" == 1 ]]; then
  python3 "$ROOT_DIR/scripts/check_multi_client_paging_log.py" "${PAGING_LOG:-$ROOT_DIR/.gate_master/serial.log}" --quota
else
  python3 "$ROOT_DIR/scripts/check_multi_client_paging_log.py" "${PAGING_LOG:-$ROOT_DIR/.gate_master/serial.log}"
fi
