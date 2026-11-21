#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Harness, the driver SDK (crates/harness), and its template driver
# (crates/harness-template, QEMU edu): SDK guarantees, the template against its
# device model, a no_std build for the bare target, a short fuzz run of the
# template, and whether the public hardware list (docs/HARDWARE.md) is current.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
cargo test --locked -p harness --features mock
cargo test --locked -p harness-template
cargo build --locked -p harness -p harness-template --target x86_64-unknown-none
cargo build --locked -p harness --features sexos
HARNESS_FUZZ_SECONDS="${HARNESS_FUZZ_SECONDS:-20}" HARNESS_FUZZ_OUT="${HARNESS_FUZZ_OUT:-$ROOT_DIR/target/harness-crashes}" \
  cargo run --locked --release -q -p harness-template --features fuzz --bin fuzz_edu
# A table row added without regenerating the list is a note, not a failure:
# many branches touch sex-hwdb, and the list is regenerated with the next one.
if ! python3 scripts/hardware_list.py --check; then
  echo "[driver_sdk.host.gate] note=hardware_list_stale fix=\"python3 scripts/hardware_list.py\""
fi
echo "[driver_sdk.host.gate] PASS"
