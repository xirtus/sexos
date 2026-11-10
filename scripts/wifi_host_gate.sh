#!/usr/bin/env bash
# Intel Wi-Fi driver core on the host: IEEE/RFC crypto vectors, WPA2/WPA3
# handshakes, chip table, firmware files (the real linux-firmware images when
# they can be fetched), DMA layout and the power-up sequence on a register model.
# No radio here: the chip itself is only proven on hardware.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
FW_DIR="$ROOT_DIR/target/wifi-firmware"
if bash scripts/fetch_wifi_firmware.sh "$FW_DIR"; then
  export SEXOS_WIFI_FIRMWARE_DIR="$FW_DIR"
else
  echo "[gate.wifi] note: firmware not fetched; real-image test skipped"
fi
cargo test -p sex-wifi --locked
echo "[gate.wifi.result] PASS scope=driver_core_host_not_radio"
