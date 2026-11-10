#!/usr/bin/env bash
# Intel Wi-Fi driver core on the host: IEEE/RFC crypto vectors, WPA2/WPA3
# handshakes, chip table, firmware files and the boot firmware pack (the real
# linux-firmware images when they can be fetched), DMA layout, the receive ring,
# the power-up sequence to ALIVE on a register model, and the init commands to
# "ready" against a firmware model (with the real x17 .pnvm when fetched), a scan,
# and the desktop's Wi-Fi status words (sex_pdx::wifi_status).
# No radio here: the chip itself is only proven on hardware.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
FW_DIR="$ROOT_DIR/target/wifi-firmware"
if bash scripts/fetch_wifi_firmware.sh "$FW_DIR"; then
  export SEXOS_WIFI_FIRMWARE_DIR="$FW_DIR"
  # The same pack the boot image carries (scripts/make_wifi_firmware_pack.py).
  python3 scripts/make_wifi_firmware_pack.py --firmware "$FW_DIR" --out "$ROOT_DIR/target/wifi.pack"
  export SEXOS_WIFI_PACK="$ROOT_DIR/target/wifi.pack"
else
  echo "[gate.wifi] note: firmware not fetched; real-image test skipped"
fi
cargo test -p sex-wifi --locked
echo "[gate.wifi.result] PASS scope=driver_core_host_not_radio"
