#!/usr/bin/env bash
# Thunderbolt 4 / USB4 host driver core on the host: control-frame CRC-32C and
# framing, NHI ring 0 (descriptors, producer/consumer words, wrap), and the
# firmware connection manager (driver ready, root router, device connected /
# disconnected, approval) against a model of Tiger Lake's NHI and ICM.
# No controller here: the x17's NHI is only proven on hardware.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
cargo test -p sex-thunderbolt --locked
if rustup target list --installed 2>/dev/null | grep -qx x86_64-unknown-none; then
  cargo build -p sex-thunderbolt --target x86_64-unknown-none --locked
fi
echo "[gate.thunderbolt.result] PASS scope=driver_core_host_not_controller"
