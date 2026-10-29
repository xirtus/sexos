#!/usr/bin/env bash
# Read the sealed build's kernel; never compile or package an image.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
python3 scripts/check_timer_pkru.py --kernel "${KERNEL:-target/x86_64-sex/release/sex-kernel}" \
  --evidence "${TIMER_PKRU_EVIDENCE:-/tmp/sexos-timer-pkru.txt}"
