#!/usr/bin/env bash
# SEXUSB_SAFE_HARDWARE_PATH_V1 gate.
# Lane 1 (tablet):    discovery + enum + producer contract + contact model.
# Lane 2 (no device): discovery summary connected=0, PD alive, no fault.
# Lane 3 (usb-kbd):   keyboard bind + forward path. BLOCKING (promoted after
#                     first PASS recorded in SEXUSB_SAFE_HARDWARE_PATH_V1.md).
# Lane 4 (usb-mouse):  Phase 3 relative boot-mouse lane. Packing already
#                     carries rel/abs in bit 32 of arg2 (audited, no change
#                     needed) — this lane just proves it end to end.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

GATE_DIR="${GATE_DIR:-/tmp/sexos_usb_path_gate}"
mkdir -p "$GATE_DIR" "$ROOT_DIR/logs"
ISO="${ISO:-sexos-v1.0.0.iso}"
QEMU_BIN="${QEMU_BIN:-qemu-system-x86_64}"
QEMU_DISPLAY="${QEMU_DISPLAY:-none}"
READY_TIMEOUT="${READY_TIMEOUT:-45}"
SETTLE_SECONDS="${SETTLE_SECONDS:-4}"
SKIP_BUILD="${SKIP_BUILD:-0}"

export SEXOS_PROOFS_DISABLED="${SEXOS_PROOFS_DISABLED:-1}"

QEMU_PID=""
cleanup() {
  set +e
  if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
    kill "$QEMU_PID" 2>/dev/null
    sleep 1
    kill -9 "$QEMU_PID" 2>/dev/null
  fi
}
trap cleanup EXIT INT TERM

has() { grep -qE "$1" "$2" 2>/dev/null; }

wait_marker() { # pattern log timeout
  local deadline=$((SECONDS + $3))
  while (( SECONDS < deadline )); do
    has "$1" "$2" && return 0
    sleep 1
  done
  return 1
}

boot_lane() { # name log qmp_sock devices...
  local name="$1" log="$2" qmp="$3"
  shift 3
  rm -f "$log" "$qmp"
  echo "[usb_path_gate] lane=$name boot"
  set +e
  # Match the master gate's CPU features; GUI availability is not an input
  # requirement, so headless is the default (override QEMU_DISPLAY if needed).
  "$QEMU_BIN" -M q35 -m 512M -cpu max,+pku -cdrom "$ISO" \
    "$@" \
    -display "$QEMU_DISPLAY" \
    -serial "file:$log" \
    -qmp "unix:$qmp,server=on,wait=off" \
    -no-reboot -no-shutdown &
  QEMU_PID=$!
  set -e
}

stop_lane() {
  set +e
  if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
    kill "$QEMU_PID" 2>/dev/null
    sleep 1
    kill -9 "$QEMU_PID" 2>/dev/null
  fi
  QEMU_PID=""
  set -e
}

qmp_py() { # sock script-on-stdin
  python3 - "$1" <<'PY'
import json, socket, sys, time
sock_path = sys.argv[1]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(sock_path)
def rd():
    buf = b""
    while b"\n" not in buf:
        buf += s.recv(4096)
    return buf
rd()
def cmd(c):
    s.sendall((json.dumps(c) + "\n").encode())
    rd()
cmd({"execute": "qmp_capabilities"})
def move(x, y):
    cmd({"execute": "input-send-event", "arguments": {"events": [
        {"type": "abs", "data": {"axis": "x", "value": x}},
        {"type": "abs", "data": {"axis": "y", "value": y}}]}})
    time.sleep(0.1)
def btn(down):
    cmd({"execute": "input-send-event", "arguments": {"events": [
        {"type": "btn", "data": {"button": "left", "down": down}}]}})
    time.sleep(0.15)
# Contact model stimulus: press, drag right+down, release.
move(8000, 8000)
btn(True)
for i in range(1, 6):
    move(8000 + i * 800, 8000 + i * 400)
btn(False)
time.sleep(0.3)
PY
}

qmp_sendkey() { # sock key
  python3 - "$1" "$2" <<'PY'
import json, socket, sys, time
sock_path, key = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(sock_path)
def rd():
    buf = b""
    while b"\n" not in buf:
        buf += s.recv(4096)
    return buf
rd()
def cmd(c):
    s.sendall((json.dumps(c) + "\n").encode())
    rd()
cmd({"execute": "qmp_capabilities"})
cmd({"execute": "send-key", "arguments": {"keys": [
    {"type": "qcode", "data": key}]}})
time.sleep(0.5)
PY
}

# Hard faults FAIL a lane. Scheduler defense markers (steal.reject /
# set_pd.null) mean the kernel caught corruption and kept running — they
# surface as WARN so the payload gets collected, not buried by a red row.
FAULT_RE='panic|PAGE FAULT|#PF|GENERAL PROTECTION|GP FAULT|#GP|DOUBLE FAULT|TRIPLE FAULT|EXCEPTION: INVALID OPCODE|EXCEPTION: UD ENTER|PKU SECURITY VIOLATION|TEST FAILED:|fault\.kill|FATAL|sched\.steal\.reject\.exhausted'
has_fault() { grep -qiE "$FAULT_RE" "$1" 2>/dev/null; }
DEFENSE_RE='sched\.steal\.reject|sched\.set_pd\.null'

# macOS ships Bash 3, which has no associative arrays. Only validated fixed
# keys reach variable indirection; printf -v assigns data without evaluating it.
row_set() {
  [[ "$1" =~ ^[a-z][a-z0-9_]*$ ]] || return 2
  printf -v "ROW_$1" '%s' "$2"
}

row_get() {
  [[ "$1" =~ ^[a-z][a-z0-9_]*$ ]] || return 2
  local row_variable="ROW_$1"
  printf '%s' "${!row_variable}"
}

# ── BUILD ────────────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" == "1" ]]; then
  row_set build "SKIP"
elif ./scripts/entrypoint_build.sh >"$GATE_DIR/build.log" 2>&1; then
  row_set build "PASS"
else
  row_set build "FAIL"
fi

# ── LANE 1: tablet ───────────────────────────────────────────────────────
L1="$GATE_DIR/lane_tablet.log"
if [[ "$(row_get build)" != "FAIL" ]]; then
  boot_lane tablet "$L1" "$GATE_DIR/qmp1.sock" \
    -device nec-usb-xhci,id=xhci -device usb-tablet,bus=xhci.0
  if wait_marker '\[sexusb\.enum\.summary\]' "$L1" "$READY_TIMEOUT"; then
    sleep "$SETTLE_SECONDS"
    qmp_py "$GATE_DIR/qmp1.sock" || true
    sleep 3
  fi
  stop_lane

  if has '\[sexusb\.discovery\.summary\] ports=[0-9]+ connected=[1-9]' "$L1"; then
    row_set usb_discovery "PASS"
  else
    row_set usb_discovery "FAIL"
  fi
  if has '\[sexusb\.enum\.summary\] ports_connected=1 slots_enabled=1 hid_devices=1 pointer_devices=1' "$L1"; then
    row_set usb_enum "PASS"
  else
    row_set usb_enum "FAIL"
  fi
  if has '\[sexinput\.usb\.contract\] op=0x260' "$L1" \
     && ! has '\[sexinput\.usb\.report\.reject\]' "$L1"; then
    row_set usb_pointer_producer "PASS"
  else
    row_set usb_pointer_producer "FAIL"
  fi
  if has '\[sexinput\.contact\.down\]' "$L1" \
     && has '\[sexinput\.contact\.drag\]' "$L1" \
     && has '\[sexinput\.contact\.up\]' "$L1"; then
    row_set touch_contact "PASS"
  else
    row_set touch_contact "FAIL"
  fi
  if has_fault "$L1"; then
    row_set lane1_fault_free "FAIL"
  elif has "$DEFENSE_RE" "$L1"; then
    row_set lane1_fault_free "WARN"
    echo "[usb_path_gate] scheduler defense fired (collect payload):"
    grep -E "$DEFENSE_RE" "$L1" | sed -n '1,3p'
  else
    row_set lane1_fault_free "PASS"
  fi
else
  for r in usb_discovery usb_enum usb_pointer_producer touch_contact lane1_fault_free; do
    row_set "$r" "SKIP"
  done
fi

# ── LANE 2: no device ────────────────────────────────────────────────────
L2="$GATE_DIR/lane_nodevice.log"
if [[ "$(row_get build)" != "FAIL" ]]; then
  boot_lane nodevice "$L2" "$GATE_DIR/qmp2.sock" \
    -device nec-usb-xhci,id=xhci
  wait_marker '\[sexusb\.discovery\.summary\]' "$L2" "$READY_TIMEOUT" || true
  sleep "$SETTLE_SECONDS"
  stop_lane

  if has '\[sexusb\.discovery\.summary\] ports=[0-9]+ connected=0 first=none ok=1' "$L2" \
     && ! has '\[sexusb\.enum\.summary\]' "$L2" \
     && ! has_fault "$L2"; then
    row_set usb_discovery_negative "PASS"
  else
    row_set usb_discovery_negative "FAIL"
  fi
else
  row_set usb_discovery_negative "SKIP"
fi

# ── LANE 3: usb-kbd (informational) ──────────────────────────────────────
L3="$GATE_DIR/lane_kbd.log"
if [[ "$(row_get build)" != "FAIL" ]]; then
  boot_lane kbd "$L3" "$GATE_DIR/qmp3.sock" \
    -device nec-usb-xhci,id=xhci -device usb-kbd,bus=xhci.0
  if wait_marker '\[sexusb\.hid\.bind\.summary\]' "$L3" "$READY_TIMEOUT"; then
    sleep "$SETTLE_SECONDS"
    qmp_sendkey "$GATE_DIR/qmp3.sock" "a" || true
    sleep 3
  fi
  stop_lane

  if has '\[sexusb\.hid\.bind\.summary\] keyboard_ep=set' "$L3" \
     && has '\[sexusb\.kbd\.forward\]' "$L3" \
     && ! has_fault "$L3"; then
    row_set usb_kbd_lane "PASS"
  else
    row_set usb_kbd_lane "FAIL"
  fi
else
  row_set usb_kbd_lane "SKIP"
fi

# ── LANE 4: usb-mouse (relative boot mouse, Phase 3) ─────────────────────
L4="$GATE_DIR/lane_mouse.log"
if [[ "$(row_get build)" != "FAIL" ]]; then
  boot_lane mouse "$L4" "$GATE_DIR/qmp4.sock" \
    -device nec-usb-xhci,id=xhci -device usb-mouse,bus=xhci.0
  if wait_marker '\[sexusb\.hid\.bind\.summary\]' "$L4" "$READY_TIMEOUT"; then
    sleep "$SETTLE_SECONDS"
    qmp_sendkey "$GATE_DIR/qmp4.sock" "a" >/dev/null 2>&1 || true
    python3 - "$GATE_DIR/qmp4.sock" <<'PY' || true
import json, socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(sys.argv[1])
def rd():
    buf = b""
    while b"\n" not in buf:
        buf += s.recv(4096)
    return buf
rd()
def cmd(c):
    s.sendall((json.dumps(c) + "\n").encode())
    rd()
cmd({"execute": "qmp_capabilities"})
for i in range(6):
    cmd({"execute": "input-send-event", "arguments": {"events": [
        {"type": "rel", "data": {"axis": "x", "value": 5}},
        {"type": "rel", "data": {"axis": "y", "value": 3}}]}})
    time.sleep(0.1)
PY
    sleep 3
  fi
  stop_lane

  if has '\[sexusb\.hid\.bind\.summary\] .*pointer_role=mouse' "$L4" \
     && has '\[sexinput\.usb_mouse\.decode\.ok\].*is_abs=false' "$L4" \
     && ! has '\[sexusb\.tablet\.abs\]' "$L4" \
     && ! has_fault "$L4"; then
    row_set usb_boot_mouse "PASS"
  else
    row_set usb_boot_mouse "FAIL"
  fi
else
  row_set usb_boot_mouse "SKIP"
fi

# ── SUMMARY ──────────────────────────────────────────────────────────────
echo ""
echo "=== USB_PATH_GATE SUMMARY ==="
EXIT=0
for r in build usb_discovery usb_enum usb_pointer_producer touch_contact \
         lane1_fault_free usb_discovery_negative usb_kbd_lane usb_boot_mouse; do
  printf '%-26s %s\n' "$r" "$(row_get "$r")"
  [[ "$(row_get "$r")" == "FAIL" ]] && EXIT=1
done
echo "lane logs: $GATE_DIR/lane_*.log"
exit "$EXIT"
