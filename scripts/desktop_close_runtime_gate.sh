#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 "$ROOT_DIR/scripts/desktop_close_gate.py" --iso "${ISO:-$ROOT_DIR/sexos-v1.0.0.iso}" --out "${GATE_DIR:-/tmp/sexos-desktop-close}" --cpu "${SEXOS_CLOSE_CPU:-max,+pku}"
