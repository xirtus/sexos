#!/usr/bin/env bash
# ARM64_PHASE_2_ENTRY_SERIAL_V1 gate.
# Exit 0 = PASS (static+build green; runtime PASS or honest SKIP_NO_QEMU).
# Never converts SKIP_NO_QEMU into runtime PASS.
set -uo pipefail
cd "$(dirname "$0")/.."
FAIL=0
note() { echo "[gate] $*"; }
fail() { echo "[gate] FAIL: $*"; FAIL=1; }

# Toolchain record: the exact 153-warning pin below is only meaningful under
# this toolchain. Re-baseline the count if rustc/cargo changes.
note "toolchain: $(rustc -Vv | head -1) / $(cargo -V)"

# ── STATIC ────────────────────────────────────────────────────────────────
note "static: target JSON parses"
python3 -c "import json;json.load(open('aarch64-sex.json'))" || fail "aarch64-sex.json invalid JSON"

note "static: linker script present"
[ -f kernel/aarch64-linker.ld ] || fail "linker script missing"

note "static: arch.rs selects exactly one current arch"
X=$(grep -c 'cfg(target_arch = "x86_64")' kernel/src/arch.rs)
A=$(grep -c 'cfg(target_arch = "aarch64")' kernel/src/arch.rs)
[ "$X" -eq 2 ] && [ "$A" -eq 2 ] || fail "arch.rs cfg arms wrong (x86=$X aarch64=$A, want 2/2)"

note "static: no std/libc/POSIX in arch/aarch64"
grep -rEn 'std::|libc|posix|pthread' kernel/src/arch/aarch64/ && fail "std/libc/posix reference found" || true

note "static: no POR/TCR2 register WRITES (Phase 6+ detection reads allowed)"
grep -rEin 'msr[[:space:]]+(tcr2_el1|por_el0|por_el1)' kernel/src/arch/aarch64/ | grep -vE ':[0-9]+:[[:space:]]*//' && fail "POR/TCR2 write present" || true

note "static: protection.rs remains unused"
grep -rn 'protection::' kernel/src | grep -v 'src/protection.rs' && fail "protection.rs gained users" || true

note "static: x86 arch module present"
[ -f kernel/src/arch/x86_64/mod.rs ] || fail "arch/x86_64/mod.rs missing"

note "static: forbidden files unchanged vs backup"
BACKUP=$(ls -t /Users/xmacbookm5/Projects/Sex_ArmV9/backups/sex_pre_phase2_*.tar.xz 2>/dev/null | head -1)
if [ -z "$BACKUP" ]; then fail "pre-phase2 backup not found"; else
  TMPD=$(mktemp -d)
  for f in kernel/src/pku.rs kernel/src/interrupts.rs kernel/src/scheduler.rs \
           kernel/src/ipc.rs kernel/src/ipc_ring.rs kernel/src/capability.rs \
           kernel/src/gdt.rs kernel/src/apic.rs kernel/src/smp.rs kernel/linker.ld \
           crates/sex-pdx/src/lib.rs crates/sex-pdx/src/ring.rs; do
    mkdir -p "$TMPD/$(dirname $f)"
    tar -xOf "$BACKUP" "Sex/$f" > "$TMPD/$f" 2>/dev/null || { fail "backup lacks $f"; continue; }
    cmp -s "$f" "$TMPD/$f" || fail "FORBIDDEN FILE CHANGED: $f"
  done
  rm -rf "$TMPD"
fi

# ── BUILD ─────────────────────────────────────────────────────────────────
note "build: x86 canonical build (cd kernel && cargo build)"
X86LOG=$(cd kernel && cargo build 2>&1)
echo "$X86LOG" | grep -q "^error" && fail "x86 build error" || true
echo "$X86LOG" | grep -q "Finished" || fail "x86 build did not finish"
echo "$X86LOG" | grep -q "generated 153 warnings" || fail "x86 warning count drifted (want 153 under pinned toolchain $(rustc -V | awk '{print $2}'); re-baseline if toolchain changed): $(echo "$X86LOG" | grep -o 'generated [0-9]* warnings' | head -1)"

note "build: aarch64 image"
scripts/build_arm64_phase2.sh >/tmp/arm64_build.log 2>&1 || fail "aarch64 build failed (see /tmp/arm64_build.log)"

note "build: ELF verification"
python3 - <<'PY' || fail "ELF verification failed"
import struct
d = open('target/aarch64-sex/release/sex-kernel-arm64','rb').read()
e_type, e_machine = struct.unpack_from('<HH', d, 16)
e_entry = struct.unpack_from('<Q', d, 24)[0]
assert e_machine == 183, f"machine {e_machine} != 183"
assert e_type == 2, f"type {e_type} != 2"
e_phoff = struct.unpack_from('<Q', d, 32)[0]
p_off  = struct.unpack_from('<Q', d, e_phoff+8)[0]
p_va   = struct.unpack_from('<Q', d, e_phoff+16)[0]
p_sz   = struct.unpack_from('<Q', d, e_phoff+32)[0]
assert p_va <= e_entry < p_va + p_sz, f"entry 0x{e_entry:x} outside text"
print(f"[gate] ELF ok: machine=183 entry=0x{e_entry:x} text=[0x{p_va:x},0x{p_va+p_sz:x})")
PY

note "build: no undefined symbols"
LLVMNM="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm"
if [ -x "$LLVMNM" ]; then
  U=$("$LLVMNM" -u target/aarch64-sex/release/sex-kernel-arm64 2>/dev/null | grep -v '^$' | wc -l | tr -d ' ')
  [ "$U" = "0" ] || fail "$U undefined symbols"
else
  note "llvm-nm unavailable; link success implies no undefined symbols"
fi

# ── RUNTIME ───────────────────────────────────────────────────────────────
RUNTIME="SKIP_NO_QEMU"
if command -v qemu-system-aarch64 >/dev/null 2>&1; then
  note "runtime: qemu-system-aarch64 found, booting"
  LOG=/tmp/arm64_phase2_qemu.log
  : > "$LOG"
  qemu-system-aarch64 -M virt,gic-version=3,virtualization=on -cpu max -smp 2 -m 512M -nographic \
      -kernel target/aarch64-sex/release/sex-kernel-arm64 > "$LOG" 2>&1 &
  QPID=$!
  sleep 8
  kill "$QPID" 2>/dev/null
  wait "$QPID" 2>/dev/null
  # ordered marker scan
  python3 - "$LOG" <<'PY' && RUNTIME="PASS" || { RUNTIME="FAIL"; fail "runtime marker scan failed"; }
import sys, re
try:
    log = open(sys.argv[1], errors='replace').read()
except FileNotFoundError:
    sys.exit(1)
order = ['[sex.arm.entry]', '[sex.arm.el1] ok=1', '[sex.arm.dtb]',
         '[sex.arm.uart] pl011=ok', 'SEX_ARM_ALIVE']
pos = -1
for m in order:
    i = log.find(m, pos + 1)
    if i < 0:
        print(f"[gate] missing marker: {m}")
        sys.exit(1)
    pos = i
for bad in ['[sex.arm.panic]', 'SEX_ARM_PHASE3_FAIL', 'SEX_ARM_PHASE4_FAIL',
            'SEX_ARM_PHASE5_FAIL', 'MISMATCH', 'FATAL', 'unexpected=1',
            'FAILCLOSED']:
    if bad in log:
        print(f"[gate] negative marker present: {bad}")
        sys.exit(1)
print("[gate] runtime markers OK")
PY
else
  note "runtime: qemu-system-aarch64 absent"
fi

echo "[gate] X86_PKU_RUNTIME_BASELINE_PENDING=1"
echo "[gate] RUNTIME=$RUNTIME"
if [ "$FAIL" -ne 0 ]; then echo "[gate] RESULT=FAIL"; exit 1; fi
echo "[gate] RESULT=PASS (runtime=$RUNTIME)"
