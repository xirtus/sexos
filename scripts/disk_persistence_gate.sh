#!/usr/bin/env bash
# DISK_PERSISTENCE_GATE — frozen product gate for the disk/data layer + text-model V2.
# Two boots sharing one NVMe image prove:
#   boot 1: linen publishes 3 disk-backed session objects, spindle `disk`
#           command lists all 3 via REAL sync DiskFS probe, quil types +
#           palette-saves to RamFS AND DiskFS ([quil.persist.save.ok]).
#   boot 2: fresh RamFS, same NVMe — quil palette-load restores the document
#           from DiskFS ([quil.persist.load.ok] with same doc, byte count and fingerprint),
#           text-color occupancy appears inside the native Quil viewport.
#           Document identity comes from matching transport fingerprints;
#           this occupancy check does not identify document glyphs.
# Marker-verified keying: every key waits for its serial-side effect, so the
# gate is robust to boot-time serial spam slowing the input path.
#
# Usage: ./scripts/disk_persistence_gate.sh          (builds ISO unless SKIP_BUILD=1)
#        GATE_DIR=/tmp/sexos_dpg ./scripts/disk_persistence_gate.sh
# QMP unix socket path must stay <108 bytes — keep GATE_DIR short.
set -uo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
GATE_DIR="${GATE_DIR:-/tmp/sexos_dpg}"
mkdir -p "$GATE_DIR"
# A failed QMP screendump must not reuse a successful older screenshot.
rm -f "$GATE_DIR/final.ppm" "$GATE_DIR/pixrows.txt" "$GATE_DIR/persistrows.txt"
ISO="${ISO:-sexos-v1.0.0.iso}"
QMP="$GATE_DIR/q.sock"
NVME="$GATE_DIR/nvme.img"
SKIP_BUILD="${SKIP_BUILD:-0}"

QEMU_PID=""
cleanup() { set +e; [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null && { kill "$QEMU_PID"; sleep 1; kill -9 "$QEMU_PID" 2>/dev/null; }; }
trap cleanup EXIT INT TERM

k() {
  python3 - "$QMP" "$1" <<'PY'
import json, socket, sys, time, os
sock_path, key = sys.argv[1], sys.argv[2]
deadline = time.time() + 10
while not os.path.exists(sock_path):
    if time.time() > deadline: sys.exit(1)
    time.sleep(0.2)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(5); s.connect(sock_path)
def rd():
    buf=b""
    while b"\n" not in buf: buf += s.recv(4096)
    return buf
rd()
def cmd(c): s.sendall((json.dumps(c)+"\n").encode()); rd()
cmd({"execute":"qmp_capabilities"})
cmd({"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":key}]}})
time.sleep(0.2)
PY
}
dump() {
  python3 - "$QMP" "$1" <<'PY'
import json, socket, sys, time
sock_path, out = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(10); s.connect(sock_path)
def rd():
    buf=b""
    while b"\n" not in buf: buf += s.recv(4096)
    return buf
rd()
def cmd(c): s.sendall((json.dumps(c)+"\n").encode()); return rd()
cmd({"execute":"qmp_capabilities"})
cmd({"execute":"screendump","arguments":{"filename":out}})
time.sleep(1)
PY
}
wait_marker() { local d=$((SECONDS+$3)); while ((SECONDS<d)); do grep -qE "$1" "$2" 2>/dev/null && return 0; sleep 1; done; return 1; }
count_re() { local c; c=$(grep -cE "$1" "$2" 2>/dev/null); echo "${c:-0}"; }
# Send key, wait until regex count in log EXCEEDS baseline. Retry once.
kv() { # kv <qcode> <regex> <log> <timeout>
  local base; base=$(count_re "$2" "$3")
  k "$1"
  local d=$((SECONDS+$4))
  while ((SECONDS<d)); do
    [[ $(count_re "$2" "$3") -gt $base ]] && return 0
    sleep 1
  done
  k "$1"
  d=$((SECONDS+$4))
  while ((SECONDS<d)); do
    [[ $(count_re "$2" "$3") -gt $base ]] && return 0
    sleep 1
  done
  echo "[dpg] key miss key=$1 re=$2"
  return 1
}
boot() {
  local log="$1"
  rm -f "$QMP"
  : > "$log"   # kill stale content so wait_marker can't match a previous run
  qemu-system-x86_64 -M q35 -m 512M -cdrom "$ISO" \
    -drive "if=none,id=nvm,file=$NVME,format=raw" \
    -device "nvme,serial=sexos01,drive=nvm" \
    -serial "file:$log" -qmp "unix:$QMP,server=on,wait=off" \
    -display none -no-reboot -no-shutdown &
  QEMU_PID=$!
}
stop() { kill "$QEMU_PID" 2>/dev/null; sleep 1; kill -9 "$QEMU_PID" 2>/dev/null; QEMU_PID=""; }

FAULT_RE='KERNEL PAGE FAULT|DOUBLE FAULT|KERNEL PANIC|GP FAULT|fault\.kill'
FAILED=0
r() { echo "ROW $1 $2"; [[ "$2" == FAIL* ]] && FAILED=1 || true; }

if [[ "$SKIP_BUILD" != "1" ]]; then
  ./scripts/entrypoint_build.sh >"$GATE_DIR/build.log" 2>&1 || { echo "ROW build FAIL"; echo "[disk.persistence.gate.result] FAIL"; exit 1; }
  echo "ROW build PASS"
else
  echo "ROW build SKIP"
fi

rm -f "$NVME"
dd if=/dev/zero of="$NVME" bs=512 count=2048 2>/dev/null

### BOOT 1
L="$GATE_DIR/b1.log"
echo "[dpg] boot 1"
boot "$L"
wait_marker '\[linen\.ready\]' "$L" 90 || echo "[dpg] WARN ready timeout b1"
# Linen's disk publish holds sexfiles through boot; user disk ops must land
# after it (a mid-publish request can be lost under IPC contention).
wait_marker '\[linen\.disk\.publish\.done\]' "$L" 300 || echo "[dpg] WARN publish timeout b1"
sleep 6

# Spindle: focus, type "disk", run — all marker-verified
kv scroll_lock 'reason=ToggleSpindle' "$L" 8
sleep 2
kv d '\[spindle\.input\.echo\.ok\] key=32' "$L" 6
kv i '\[spindle\.input\.echo\.ok\] key=23' "$L" 6
kv s '\[spindle\.input\.echo\.ok\] key=31' "$L" 6
kv k '\[spindle\.input\.echo\.ok\] key=37' "$L" 6
kv ret '\[spindle\.disk\.command\]' "$L" 20
kv scroll_lock 'reason=ToggleSpindle' "$L" 8
sleep 1

# Quil: open via shell palette idx1, type "hi", palette save (row1)
kv grave_accent '\[shell\.palette\.item\] idx=0' "$L" 8
kv j '\[shell\.palette\.select\] old=0 new=1' "$L" 8
kv ret '\[shell\.palette\.exec\] idx=1' "$L" 10
sleep 2
kv esc '\[quil\.palette\.action\] kind=esc' "$L" 8
kv h '\[quil\.text\.recv\]' "$L" 6 || true
k i; sleep 1
kv esc 'kind=esc toggle_on=1' "$L" 8
kv down '\[quil\.palette\.selected\] row=1' "$L" 8
k ret
wait_marker '\[quil\.persist\.save\.(ok|err)' "$L" 180 || echo "[dpg] WARN persist save timeout"
sleep 2
stop

### BOOT 2 — same NVMe, fresh RamFS
L2="$GATE_DIR/b2.log"
echo "[dpg] boot 2"
boot "$L2"
wait_marker '\[linen\.ready\]' "$L2" 90 || echo "[dpg] WARN ready timeout b2"
wait_marker '\[linen\.disk\.publish\.done\]' "$L2" 300 || echo "[dpg] WARN publish timeout b2"
sleep 6
kv grave_accent '\[shell\.palette\.item\] idx=0' "$L2" 8
kv j '\[shell\.palette\.select\] old=0 new=1' "$L2" 8
kv ret '\[shell\.palette\.exec\] idx=1' "$L2" 10
sleep 2
kv down '\[quil\.palette\.selected\] row=1' "$L2" 8
kv down '\[quil\.palette\.selected\] row=2' "$L2" 8
k ret
wait_marker '\[quil\.persist\.load\.(ok|err|miss)' "$L2" 180 || echo "[dpg] WARN persist load timeout"
sleep 2

# LINEN_DISK_OPEN_V1: open the disk-backed quil doc from the Linen list —
# 3rd entry (disk-nquil-v1) → Enter → shell intent → quil PD DiskFS load.
kv grave_accent '\[shell\.palette\.item\] idx=0' "$L2" 8
kv j '\[shell\.palette\.select\] old=0 new=1' "$L2" 8
kv j '\[shell\.palette\.select\] old=1 new=2' "$L2" 8
kv ret '\[shell\.palette\.exec\] idx=2' "$L2" 10
sleep 3
kv j '\[linen\.nav\.select\.ok\]|\[shell\.action\.select_next_linen\]' "$L2" 8
kv j '\[linen\.nav\.select\.ok\]|\[shell\.action\.select_next_linen\]' "$L2" 8
k ret
wait_marker '\[quil\.open\.disk_doc\.recv\]' "$L2" 30 || echo "[dpg] WARN no disk_doc recv"
# wait for the SECOND persist load completion (first was the palette load)
d=$((SECONDS+240)); while ((SECONDS<d)); do
  [[ $(count_re '\[quil\.persist\.load\.(ok|err|miss)' "$L2") -ge 2 ]] && break
  sleep 2
done
sleep 2
dump "$GATE_DIR/final.ppm"; sleep 1
stop

### Rows
has1() { grep -qE "$1" "$GATE_DIR/b1.log" 2>/dev/null; }
has2() { grep -qE "$1" "$GATE_DIR/b2.log" 2>/dev/null; }

has1 '\[linen\.disk\.publish\.done\] count=3' && r linen_disk_publish_b1 PASS || r linen_disk_publish_b1 FAIL
has2 '\[linen\.disk\.publish\.done\] count=3' && r linen_disk_publish_b2 PASS || r linen_disk_publish_b2 FAIL
has1 '\[spindle\.disk\.command\] found=3 ok=1' && r spindle_disk_cmd PASS || r spindle_disk_cmd FAIL
has1 '\[quil\.persist\.save\.ok\]' && r quil_persist_save PASS || r quil_persist_save FAIL
has2 '\[quil\.persist\.load\.ok\]' && r quil_persist_load_reboot PASS || r quil_persist_load_reboot FAIL

# Both palette Load and Linen Open must restore the saved document's length
# and FNV-1a fingerprint. Missing/malformed evidence is a failure.
python3 scripts/check_disk_persistence.py logs "$GATE_DIR/b1.log" "$GATE_DIR/b2.log" \
  >"$GATE_DIR/persistrows.txt" || FAILED=1
cat "$GATE_DIR/persistrows.txt"

has2 '\[linen\.quil\.disk_doc\.intent\].*ok=1' && r linen_disk_open_intent PASS || r linen_disk_open_intent FAIL
has2 '\[quil\.open\.disk_doc\.recv\]' && r quil_disk_doc_recv PASS || r quil_disk_doc_recv FAIL
[[ $(count_re '\[quil\.persist\.load\.ok\]' "$GATE_DIR/b2.log") -ge 2 ]] && r quil_disk_doc_load PASS || r quil_disk_doc_load FAIL
grep -qE "$FAULT_RE" "$GATE_DIR/b1.log" && r fault_free_b1 FAIL || r fault_free_b1 PASS
grep -qE "$FAULT_RE" "$GATE_DIR/b2.log" && r fault_free_b2 FAIL || r fault_free_b2 PASS
./scripts/rsp0_regression_gate.sh "$GATE_DIR/b1.log" >/dev/null 2>&1 && r rsp0_gate PASS || r rsp0_gate FAIL

# Pixel occupancy only: inspect the current shell-authorized Quil viewport
# for text color 0xE0F0FF. Fingerprints above establish document identity.
python3 scripts/check_disk_persistence.py pixels "$GATE_DIR/final.ppm" --log "$GATE_DIR/b2.log" \
  >"$GATE_DIR/pixrows.txt" || FAILED=1
cat "$GATE_DIR/pixrows.txt"
grep -q " FAIL" "$GATE_DIR/pixrows.txt" && FAILED=1

if [[ "$FAILED" == "0" ]]; then
  echo "[disk.persistence.gate.result] PASS"
else
  echo "[disk.persistence.gate.result] FAIL"
  exit 1
fi
echo "[dpg] logs: $GATE_DIR/b1.log $GATE_DIR/b2.log"
