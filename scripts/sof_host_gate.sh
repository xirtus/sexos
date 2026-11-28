#!/usr/bin/env bash
# Intel audio DSP + Sound Open Firmware core (crates/sex-sof): register model,
# boot sequence, IPC3 bytes and the DMIC capture graph, checked on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
cargo test --locked -p sex-sof
echo "[sof.host.gate] PASS"
