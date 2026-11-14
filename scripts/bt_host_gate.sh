#!/usr/bin/env bash
# Cufflinks (Bluetooth) on the host: HCI goldens, Security Manager spec vectors
# (AES-CMAC, f4/f5/f6/g2, P-256), the Intel firmware loader against a synthetic .sfi
# and a scripted controller (plus the real linux-firmware images when they can be
# fetched), a simulated controller through bring-up, scan and LE Secure Connections
# pairing, and the sex_pdx::bt_hci ring.
# No radio here: the controller itself is only proven on hardware (x17 receipt).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
FW_DIR="$ROOT_DIR/target/bt-firmware"
if bash scripts/fetch_bt_firmware.sh "$FW_DIR"; then
  export SEXOS_BT_FIRMWARE_DIR="$FW_DIR"
else
  echo "[gate.bt] note: firmware not fetched; real-image test skipped"
fi
cargo test -p sex-bt --locked
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
rustc --edition=2021 --crate-name bt_hci --test crates/sex-pdx/src/bt_hci.rs -o "$TMP/bt_hci" && "$TMP/bt_hci"
echo "[gate.bt.result] PASS scope=host_stack_not_radio"
