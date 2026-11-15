#!/usr/bin/env bash
# Cufflinks (Bluetooth) on the host: HCI goldens, Security Manager spec vectors
# (AES-CMAC, f4/f5/f6/g2, P-256), the Intel firmware loader against a synthetic .sfi
# and a scripted controller (plus the real linux-firmware images when they can be
# fetched), a simulated controller through bring-up, scan and LE Secure Connections
# pairing, ATT/GATT and report-map goldens, a simulated BLE keyboard (HID over GATT)
# and classic HID mouse end to end, bonds, and the sex_pdx::bt_hci ring, HID mailbox
# and bt_bonds store. W3: the SBC encoder against an independent reference decoder
# (header, CRC, SNR in every mode), SDP, AVDTP and RTP, ACL flow control, a simulated
# A2DP headset end to end (pair, SDP, AVDTP setup both ways, the tone it decodes,
# suspend/resume/close, malformed peers), and the sex_pdx::bt_audio ring.
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
printf '#[path = "%s/crates/sex-pdx/src/bt_hci.rs"] pub mod bt_hci;\n#[path = "%s/crates/sex-pdx/src/bt_bonds.rs"] pub mod bt_bonds;\n#[path = "%s/crates/sex-pdx/src/bt_audio.rs"] pub mod bt_audio;\n' \
  "$ROOT_DIR" "$ROOT_DIR" "$ROOT_DIR" > "$TMP/pdx_bt.rs"
rustc --edition=2021 --crate-name pdx_bt --test "$TMP/pdx_bt.rs" -o "$TMP/pdx_bt" && "$TMP/pdx_bt"
echo "[gate.bt.result] PASS scope=host_stack_not_radio"
