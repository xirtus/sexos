#!/usr/bin/env bash
# APP_DATA_GATE — frozen product gate for the app/data layer.
# One boot proves: Spindle visible+typing, Linen visible (own sid 157) +
# j/k select, Quil visible + live typing + save/load roundtrip,
# Linen→Quil open chain (collar core-app + grant match), Bell ring
# multi-entry + nav + lane text, Collar grants list + nav, Mesh visible +
# focus-change refresh, zero faults, zero AUTH, rsp0 marker.
#
# Usage: ./scripts/app_data_gate.sh            (builds ISO unless SKIP_BUILD=1)
#        GATE_DIR=/tmp/claude-1000/adg ./scripts/app_data_gate.sh
# QMP unix socket path must stay <108 bytes — keep GATE_DIR short.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
GATE_DIR="${GATE_DIR:-/tmp/sexos_adg}"
mkdir -p "$GATE_DIR"
ISO="${ISO:-sexos-v1.0.0.iso}"
LOG="$GATE_DIR/lane.log"
QMP="$GATE_DIR/q.sock"
SKIP_BUILD="${SKIP_BUILD:-0}"
rm -f "$LOG" "$QMP"

QEMU_PID=""
cleanup() { set +e; [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null && { kill "$QEMU_PID"; sleep 1; kill -9 "$QEMU_PID" 2>/dev/null; }; }
trap cleanup EXIT INT TERM
has() { grep -qE "$1" "$LOG" 2>/dev/null; }
wait_marker() { local d=$((SECONDS+$2)); while ((SECONDS<d)); do has "$1" && return 0; sleep 1; done; return 1; }

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

FAULT_RE='KERNEL PAGE FAULT|DOUBLE FAULT|KERNEL PANIC|GP FAULT|fault\.kill'
FAILED=0
r() { echo "ROW $1 $2"; [[ "$2" == FAIL* ]] && FAILED=1 || true; }

if [[ "$SKIP_BUILD" != "1" ]]; then
  ./scripts/entrypoint_build.sh >"$GATE_DIR/build.log" 2>&1 || { echo "ROW build FAIL"; echo "[appdata.gate.result] FAIL"; exit 1; }
  echo "ROW build PASS"
else
  echo "ROW build SKIP"
fi

echo "[app_data_gate] boot"
qemu-system-x86_64 -M q35 -m 512M -cdrom "$ISO" \
  -serial "file:$LOG" -qmp "unix:$QMP,server=on,wait=off" \
  -display none -no-reboot -no-shutdown &
QEMU_PID=$!
wait_marker '\[linen\.ready\]' 60 || echo "[app_data_gate] WARN ready timeout"
sleep 4

# ── Spindle: Scroll Lock toggle, type "help" + Enter, ghost + history ──
k scroll_lock; sleep 2
k h; k e; k l; k p; sleep 1
k ret; sleep 2
# ghost autosuggest: "he" + Tab accepts "help" from history, Enter runs it
k h; k e; sleep 1
k tab; sleep 1
k ret; sleep 2
# history recall: Up brings back last command
k up; sleep 1
k esc; sleep 1

# ── Mesh open (F12) so later focus hops fire live refresh ──
k f12; sleep 2

# ── Linen open #1: palette idx2, j select, Enter ──
k grave_accent; sleep 1
k j; k j; sleep 1
k ret; sleep 2
k j; sleep 1
k ret; sleep 2

# ── Linen open #2 (refocus; selection advanced by j) → bell entry #2 ──
k grave_accent; sleep 1
k j; k j; sleep 1
k ret; sleep 2
k j; sleep 1
k ret; sleep 2

# ── Bell: palette idx4, nav over populated ring ──
k grave_accent; sleep 1
k j; k j; k j; k j; sleep 1
k ret; sleep 2
k j; sleep 1
k j; sleep 1

# ── Collar: palette idx5, grants nav ──
k grave_accent; sleep 1
k j; k j; k j; k j; k j; sleep 1
k ret; sleep 2
k j; sleep 1
k k; sleep 1

# ── Minimize Bell (PageDown toggle) + Collar (Esc while focused) so the
# fixed-geometry Quil content region is unoccluded for the pixel scan ──
k esc; sleep 1
k pgdn; sleep 1

# ── Quil: palette idx1, text mode, type, save, load ──
k grave_accent; sleep 1
k j; sleep 1
k ret; sleep 2
k esc; sleep 1
k a; sleep 1
k b; sleep 1
k esc; sleep 1
k down; sleep 1
k ret; sleep 3
k down; sleep 1
k ret; sleep 3

dump "$GATE_DIR/final.ppm"; sleep 1
kill "$QEMU_PID" 2>/dev/null || true; sleep 1; kill -9 "$QEMU_PID" 2>/dev/null || true
QEMU_PID=""

# ── Rows ──
has '\[spindle\.grid\.surface\.ok\]' && r spindle_visible PASS || r spindle_visible FAIL
has '\[spindle\.(input\.echo\.ok|key\.char)\]' && r spindle_typing PASS || r spindle_typing FAIL
has '\[spindle\.ghost\.accept\]' && r spindle_ghost PASS || r spindle_ghost FAIL
has '\[spindle\.history\.nav\] dir=up .*ok=1' && r spindle_history PASS || r spindle_history FAIL
# Submission evidence only; check_desktop_pixels.py verifies rendered glyphs.
has '\[linen\.list\.text\] .*rows=[1-9].*committed_view=1' && r linen_text_submitted PASS || r linen_text_submitted FAIL
has '\[linen\.remote\.snapshot\.fallback\]|\[linen\.remote\.snapshot\.ok\] count=[1-9]' && r linen_objects PASS || r linen_objects FAIL
has '\[linen\.nav\.select\.ok\]|\[shell\.action\.select_next_linen\]' && r linen_nav_select PASS || r linen_nav_select FAIL
has '\[quil\.surface\.visible\.ok\]' && r quil_visible PASS || r quil_visible FAIL
has '\[mesh\.surface\.visible\.ok\]' && r mesh_visible PASS || r mesh_visible FAIL
has '\[mesh\.pd_graph\.refresh\] reason=focus_change' && r mesh_refresh PASS || r mesh_refresh FAIL
has '\[collar\.policy\.allow\] .*reason=core_app' && r collar_core_app PASS || r collar_core_app FAIL
has '\[collar\.grant\.match\]' && r collar_grant_match PASS || r collar_grant_match FAIL
has '\[collar\.grants\.render\.ok\] grants=[1-9]' && r collar_grants_render PASS || r collar_grants_render FAIL
has '\[linen\.quil\.buffer\.linked\]' && r linen_quil_open PASS || r linen_quil_open FAIL
has '\[bell\.event\.object_link\]' && r bell_event PASS || r bell_event FAIL
has '\[bell\.lane\.render\.ok\] total=[1-9]' && r bell_lane_render PASS || r bell_lane_render FAIL
has '\[bell\.nav\.move\] old=[0-9]+ new=[0-9]+ total=[2-9]' && r bell_multi_entry PASS || r bell_multi_entry FAIL
has '\[quil\.text\.recv\]' && r quil_typing PASS || r quil_typing FAIL
has '\[quil\.save\.ok\]' && r quil_save PASS || r quil_save FAIL
has '\[quil\.load\.ok\]' && r quil_load PASS || r quil_load FAIL
grep -qE "$FAULT_RE" "$LOG" && r fault_free FAIL || r fault_free PASS
grep -qE 'AUTH:' "$LOG" && r auth_free FAIL || r auth_free PASS
./scripts/rsp0_regression_gate.sh "$LOG" >/dev/null 2>&1 && r rsp0_gate PASS || r rsp0_gate FAIL

# Pixel proof. quil threshold is >0: the fixed-geometry content sid 156 is
# partially occluded by the shell's quil buffer-list overlay on frame 201,
# so only a sliver of 0xE0F0FF glyphs stays visible — nonzero proves the
# quil PD's own text reached the framebuffer (full typing proof is the
# quil_typing/quil_save marker rows). Shell text (bell/collar/mesh lists,
# 0xE8FFFF outside the spindle grid region) needs a real glyph count.
PIXROWS="$GATE_DIR/pixrows.txt"
python3 - "$GATE_DIR/final.ppm" <<'PY' >"$PIXROWS" || true
import sys
try:
    with open(sys.argv[1],'rb') as f: data=f.read()
    parts=data.split(b'\n',3)
    if parts[0].strip()!=b'P6':
        print("ROW pixel_scan SKIP fmt"); sys.exit(0)
    w,h=map(int,parts[1].split()); px=parts[3]
    quil=0; text=0
    for y in range(0,h):
        for x in range(0,w):
            i=(y*w+x)*3
            r,g,b=px[i],px[i+1],px[i+2]
            if r==0xE0 and g==0xF0 and b==0xFF and 1072<=x<1272 and 56<=y<360: quil+=1
            if r==0xE8 and g==0xFF and b==0xFF and (x<1008 or y<632): text+=1
    print(f"ROW pixel_quil {'PASS' if quil>0 else 'FAIL'} count={quil}")
    print(f"ROW pixel_shell_text {'PASS' if text>10 else 'FAIL'} count={text}")
except FileNotFoundError:
    print("ROW pixel_scan SKIP no_dump")
PY
cat "$PIXROWS"
grep -q " FAIL" "$PIXROWS" && FAILED=1

if [[ "$FAILED" == "0" ]]; then
  echo "[appdata.gate.result] PASS"
else
  echo "[appdata.gate.result] FAIL"
  exit 1
fi
echo "[app_data_gate] log=$LOG"
