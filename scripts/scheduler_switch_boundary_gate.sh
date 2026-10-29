#!/usr/bin/env bash
# Read the sealed build's kernel; this gate never builds an image.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
python3 scripts/check_scheduler_switch.py --kernel "${KERNEL:-target/x86_64-sex/release/sex-kernel}" \
  --evidence "${SWITCH_EVIDENCE:-/tmp/sexos-scheduler-switch.txt}"
