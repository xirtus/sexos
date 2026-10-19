#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

QEMU="${QEMU_BIN:-/opt/homebrew/bin/qemu-system-x86_64}"
ISO="${SEXOS_ISO:-}"
if [ -z "$ISO" ]; then
    for candidate in "$ROOT_DIR/sexos-v0.1.0.iso" "$ROOT_DIR/sexos-v1.0.0.iso"; do
        if [ -f "$candidate" ]; then ISO="$candidate"; break; fi
    done
fi
if [ -z "$ISO" ] || [ ! -f "$ISO" ]; then
    echo "error: no SexOS ISO found. Download one into the repo root or set SEXOS_ISO=/path/to.iso" >&2
    exit 1
fi

echo "🧪 Booting SexOS on QEMU (M1 → x86_64 emulation)"
echo "   ISO: $ISO"
echo ""

# TCG emulation (no HVF on M1 for x86_64 guests)
"$QEMU" \
  -M q35 \
  -m 512M \
  -cpu max \
  -cdrom "$ISO" \
  -serial stdio \
  -display cocoa \
  -device nec-usb-xhci,id=xhci \
  -device usb-kbd,bus=xhci.0 \
  -device usb-tablet,bus=xhci.0 \
  -no-reboot
