#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --locked -p moan-core
# Qualify the waveform checks against the real DSP before booting QEMU.
MOAN_TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$MOAN_TEST_DIR"' EXIT
cargo run --locked --quiet -p moan-core --example effects_reference -- "$MOAN_TEST_DIR/effects.wav"
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 - "$MOAN_TEST_DIR/effects.wav" <<'PY'
import sys
from pathlib import Path
from moan_gate import inspect_effects
print(inspect_effects(Path(sys.argv[1])))
PY
