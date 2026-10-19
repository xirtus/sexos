#!/usr/bin/env bash
set -e

CMD="${1:-run}"
PRINT_CMD_ONLY="${QEMU_PRINT_CMD:-0}"
TRACE_ARGS=""
if [ -n "$SEXUSB_XHCI_TRACE" ]; then
    TRACE_ARGS="-trace usb_xhci_slot_address -trace usb_xhci_queue_event -trace usb_xhci_fetch_trb -trace usb_xhci_doorbell"
fi

# Select USB device: tablet-display-sdl (default), mouse, tablet, or kbd.
# tablet-display-sdl binds SDL mouse events directly to USB HID, enabling cursor input.
# Override: SEXUSB_QEMU_DEVICE=mouse to revert to boot-HID-only probe mode.
USB_DEV="${SEXUSB_QEMU_DEVICE:-tablet-display-sdl}"
case "$USB_DEV" in
  mouse)  USB_DEVICE_ARG="-device usb-mouse,bus=xhci.0" ;;
  tablet) USB_DEVICE_ARG="-device usb-tablet,bus=xhci.0" ;;
  tablet-display-sdl) USB_DEVICE_ARG="-device usb-tablet,bus=xhci.0,display=sdl" ;;
  kbd)    USB_DEVICE_ARG="-device usb-kbd,bus=xhci.0" ;;
  kbd-display-sdl) USB_DEVICE_ARG="-device usb-kbd,bus=xhci.0,display=sdl" ;;
  kbd+tablet) USB_DEVICE_ARG="-device usb-kbd,bus=xhci.0 -device usb-tablet,bus=xhci.0" ;;
  *)
    echo "error: unknown SEXUSB_QEMU_DEVICE=$USB_DEV (use mouse, tablet, tablet-display-sdl, kbd, kbd+tablet)"
    exit 1
    ;;
esac

# Select QEMU display/input backend.
DISPLAY_MODE="${SEXOS_QEMU_DISPLAY:-sdl}"
case "$DISPLAY_MODE" in
  sdl)      DISPLAY_ARG="-display sdl" ;;
  sdl-grab) DISPLAY_ARG="-display sdl,grab-mod=lctrl-lalt" ;;
  gtk)      DISPLAY_ARG="-display gtk" ;;
  gtk-grab) DISPLAY_ARG="-display gtk,grab-on-hover=on" ;;
  none)     DISPLAY_ARG="-display none" ;;
  *)
    echo "error: unknown SEXOS_QEMU_DISPLAY=$DISPLAY_MODE (use sdl, sdl-grab, gtk, gtk-grab, none)"
    exit 1
    ;;
esac

# Optional: disable QEMU default devices (PS/2 keyboard+mouse) to avoid input conflicts.
NODEFAULTS_ARG=""
if [ -n "$SEXOS_QEMU_NODEFAULTS" ]; then
    NODEFAULTS_ARG="-nodefaults"
    echo "QEMU nodefaults: enabled (PS/2 input devices disabled)"
fi

# QMP monitor socket for deterministic input injection (dev infra only).
QMP_ARG=""
QMP_SOCK=""
cleanup_qmp_sock() {
    if [ -n "${QMP_SOCK:-}" ] && [ -e "$QMP_SOCK" ]; then
        rm -f "$QMP_SOCK"
    fi
}
if [ -n "$SEXOS_QEMU_INPUT_INJECT" ] || [ -n "$SEXOS_QEMU_QMP" ]; then
    if [ -n "${SEXOS_QMP_SOCK:-}" ]; then
        QMP_SOCK="$SEXOS_QMP_SOCK"
    else
        QMP_SOCK="/tmp/sexos-qmp-${USER:-sexos}-$$.sock"
    fi
    cleanup_qmp_sock
    trap cleanup_qmp_sock EXIT INT TERM
    QMP_ARG="-qmp unix:${QMP_SOCK},server=on,wait=off"
    echo "QMP monitor: $QMP_SOCK"
    echo "[dev.qmp.arg] sock=$QMP_SOCK ok=1"
    echo "[dev.qmp.cmd] $QMP_ARG"
fi

# Optional: disable QEMU i8042/PS2 controller so host keyboard events route to USB HID.
MACHINE_ARG="-M q35"
if [ "${SEXOS_QEMU_I8042:-}" = "off" ]; then
    MACHINE_ARG="-machine q35,i8042=off"
    echo "QEMU i8042: disabled (PS/2 controller off, USB HID receives keyboard)"
fi

# Assemble QEMU arguments.
# QEMU_BIN override: use QEMU_BIN=/path/to/qemu-system-x86_64 to test different versions
QEMU_BIN="${QEMU_BIN:-qemu-system-x86_64}"
ISO="${SEXOS_ISO:-}"
if [ -z "$ISO" ]; then
    for candidate in sexos-v0.1.0.iso sexos-v1.0.0.iso; do
        if [ -f "$candidate" ]; then ISO="$candidate"; break; fi
    done
fi
if [ -z "$ISO" ] || [ ! -f "$ISO" ]; then
    echo "error: no SexOS ISO found. Download one into the repo root or set SEXOS_ISO=/path/to.iso" >&2
    exit 1
fi
QEMU_ARGS=(
    $MACHINE_ARG
    -m 512M
    -cpu max,+pku
    -cdrom "$ISO"
    -device nec-usb-xhci,id=xhci
    $USB_DEVICE_ARG
    $TRACE_ARGS
    -serial stdio
    $DISPLAY_ARG
    $NODEFAULTS_ARG
    $QMP_ARG
)

emit_qemu_input_diag() {
    echo "[qemu.input.args] display=$DISPLAY_MODE usb_device=$USB_DEV args=\"${DISPLAY_ARG} ${USB_DEVICE_ARG}\""
    echo "[qemu.input.usb] mode=$USB_DEV args=\"${USB_DEVICE_ARG}\""
    echo "[qemu.input.binding] mode=$USB_DEV ok=1"
}

if [ "$PRINT_CMD_ONLY" = "1" ]; then
    echo "=== QEMU_PRINT_CMD=1 ==="
    echo "QEMU binary: $QEMU_BIN"
    echo "USB device:  $USB_DEV"
    echo "Display:     $DISPLAY_MODE"
    echo "Nodefaults:  ${SEXOS_QEMU_NODEFAULTS:-0}"
    echo "i8042:       ${SEXOS_QEMU_I8042:-default}"
    echo "XHCI trace:  ${SEXUSB_XHCI_TRACE:-0}"
    echo "QMP inject:  ${SEXOS_QEMU_INPUT_INJECT:-0}"
    echo "QMP:         ${SEXOS_QEMU_QMP:-0}"
    if [ -n "$QMP_SOCK" ]; then
        echo "QMP sock:    $QMP_SOCK"
    fi
    emit_qemu_input_diag
    echo ""
    echo "=== Single-line command ==="
    echo "$QEMU_BIN ${QEMU_ARGS[*]}"
    echo ""
    echo "=== One-arg-per-line ==="
    printf '%s\n' "$QEMU_BIN" "${QEMU_ARGS[@]}"
    exit 0
fi

emit_qemu_input_diag

case "$CMD" in
  run)
    "$QEMU_BIN" "${QEMU_ARGS[@]}"
    ;;
  run-nographic)
    QEMU_NOGRAPHIC_ARGS=(
      $MACHINE_ARG
      -m 512M
      -cpu max,+pku
      -cdrom "$ISO"
      -device nec-usb-xhci,id=xhci
      $USB_DEVICE_ARG
      $TRACE_ARGS
      -display none
      -serial stdio
      $NODEFAULTS_ARG
      $QMP_ARG
    )
    "$QEMU_BIN" "${QEMU_NOGRAPHIC_ARGS[@]}"
    ;;
  *)
    echo "usage: ./dev.sh [run|run-nographic]"
    echo ""
    echo "Environment variables:"
    echo "  SEXUSB_QEMU_DEVICE=mouse|tablet|tablet-display-sdl|kbd  (default: tablet-display-sdl)"
    echo "  SEXOS_QEMU_DISPLAY=sdl|sdl-grab|gtk|gtk-grab|none   (default: sdl)"
    echo "  SEXOS_QEMU_NODEFAULTS=1          disable PS/2 defaults"
    echo "  SEXOS_QEMU_I8042=off              disable i8042/PS2 controller (USB HID input)"
    echo "  SEXOS_QEMU_QMP=1                  enable QMP socket (for deterministic input injection)"
    echo "  SEXOS_QEMU_INPUT_INJECT=1         alias for SEXOS_QEMU_QMP=1"
    echo "  SEXOS_QMP_SOCK=/tmp/path.sock     explicit QMP socket path (default: /tmp/sexos-qmp-\${USER:-sexos}-\$\$.sock)"
    echo "  SEXUSB_XHCI_TRACE=1              enable xHCI trace events"
    echo "  QEMU_PRINT_CMD=1                 print argv and exit"
    exit 1
    ;;
esac
