#!/usr/bin/env bash
# HISTORICAL: scope-pinned to the pre-PHASE-3 backup; later phases
# legitimately changed pinned files. For the CURRENT tree use
# scripts/arm64_phase5_gate.sh (superset of all static+build checks).
# ARM64_PHASE_3_MMU_GLOBAL_VAS_V1 gate.
# Exit 0 = PASS (static+build green; runtime PASS or honest SKIP_NO_QEMU).
# Never converts SKIP_NO_QEMU into runtime PASS.
set -uo pipefail
cd "$(dirname "$0")/.."
FAIL=0
note() { echo "[gate3] $*"; }
fail() { echo "[gate3] FAIL: $*"; FAIL=1; }

# Toolchain record: the exact 153-warning pin below is only meaningful under
# this toolchain. Re-baseline the count if rustc/cargo changes.
note "toolchain: $(rustc -Vv | head -1) / $(cargo -V)"

# ── STATIC ────────────────────────────────────────────────────────────────
note "static: no std/libc/POSIX in arch/aarch64"
grep -rEn 'std::|libc|posix|pthread' kernel/src/arch/aarch64/ && fail "std/libc/posix reference found" || true

note "static: no POE/S1PIE/TCR2/POR/E0POE implementation (docs may defer)"
grep -rEin 's1poe|s1pie|tcr2|por_el|e0poe' kernel/src/arch/aarch64/ \
  | grep -vE ':[0-9]+:[[:space:]]*//' \
  | grep -vE ':[0-9]+:.*//!' \
  && fail "POE/TCR2/POR code present in Phase 3" || true

note "static: no CPACR_EL1 write with any bit set (E0POE-adjacent safety)"
grep -rn 'CPACR_EL1' kernel/src/arch/aarch64/ | grep -vE ':[0-9]+:[[:space:]]*//' \
  | grep -v 'xzr' | grep -v 'CPACR_EL1=0' \
  && fail "unexpected CPACR_EL1 manipulation" || true

note "static: protection.rs remains unused"
grep -rn 'protection::' kernel/src | grep -v 'src/protection.rs' && fail "protection.rs gained users" || true

note "static: forbidden x86 files byte-identical vs PRE-PHASE2 backup"
B2=$(ls -t /Users/xmacbookm5/Projects/Sex_ArmV9/backups/sex_pre_phase2_*.tar.xz 2>/dev/null | head -1)
if [ -z "$B2" ]; then fail "pre-phase2 backup not found"; else
  TMPD=$(mktemp -d)
  for f in kernel/src/pku.rs kernel/src/interrupts.rs kernel/src/scheduler.rs \
           kernel/src/ipc.rs kernel/src/ipc_ring.rs kernel/src/capability.rs \
           kernel/src/gdt.rs kernel/src/apic.rs kernel/src/smp.rs kernel/linker.ld \
           crates/sex-pdx/src/lib.rs crates/sex-pdx/src/ring.rs; do
    mkdir -p "$TMPD/$(dirname $f)"
    tar -xOf "$B2" "Sex/$f" > "$TMPD/$f" 2>/dev/null || { fail "backup lacks $f"; continue; }
    cmp -s "$f" "$TMPD/$f" || fail "FORBIDDEN FILE CHANGED: $f"
  done
  rm -rf "$TMPD"
fi

note "static: Phase-3 scope pin — files outside arch/aarch64 unchanged vs PRE-PHASE3 backup"
B3=$(ls -t /Users/xmacbookm5/Projects/Sex_ArmV9/backups/sex_pre_phase3_*.tar.xz 2>/dev/null | head -1)
if [ -z "$B3" ]; then fail "pre-phase3 backup not found"; else
  TMPD=$(mktemp -d)
  for f in kernel/src/lib.rs kernel/src/main.rs kernel/src/arch.rs kernel/Cargo.toml \
           kernel/src/arch/aarch64/main.rs kernel/src/arch/aarch64/serial.rs \
           kernel/aarch64-linker.ld aarch64-sex.json; do
    mkdir -p "$TMPD/$(dirname $f)"
    tar -xOf "$B3" "Sex/$f" > "$TMPD/$f" 2>/dev/null || { fail "phase3 backup lacks $f"; continue; }
    cmp -s "$f" "$TMPD/$f" || fail "OUT-OF-SCOPE FILE CHANGED IN PHASE 3: $f"
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
scripts/build_arm64_phase2.sh >/tmp/arm64_p3_build.log 2>&1 || fail "aarch64 build failed (see /tmp/arm64_p3_build.log)"

note "build: ELF + symbols verification"
python3 - <<'PY' || fail "ELF verification failed"
import struct, subprocess
d = open('target/aarch64-sex/release/sex-kernel-arm64','rb').read()
e_type, e_machine = struct.unpack_from('<HH', d, 16)
e_entry = struct.unpack_from('<Q', d, 24)[0]
assert e_machine == 183, f"machine {e_machine} != 183"
assert e_type == 2, f"type {e_type} != 2"
assert e_entry == 0x40080000, f"entry 0x{e_entry:x} != 0x40080000"
sysroot = subprocess.check_output(['rustc','--print','sysroot']).decode().strip()
nm = subprocess.run([f"{sysroot}/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm",
                     'target/aarch64-sex/release/sex-kernel-arm64'],
                    capture_output=True, text=True)
syms = {l.split()[2]: int(l.split()[0],16) for l in nm.stdout.splitlines() if len(l.split())>=3}
need = ['sex_arm_vectors','sex_arm_el0_enter','sex_arm_rust_el0_warden',
        'sex_arm_rust_el1_warden','__kernel_start','__kernel_end','__bss_start']
for s in need:
    assert s in syms, f"missing symbol {s}"
assert syms['sex_arm_vectors'] % 2048 == 0, "vectors not 2048-aligned"
assert syms['__bss_start'] % 4096 == 0, "__bss_start not page-aligned (text split unsafe)"
u = subprocess.run([f"{sysroot}/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm", '-u',
                    'target/aarch64-sex/release/sex-kernel-arm64'], capture_output=True, text=True)
undef = [l for l in u.stdout.splitlines() if l.strip()]
assert not undef, f"undefined symbols: {undef[:5]}"
print(f"[gate3] ELF ok: entry=0x{e_entry:x} vectors@0x{syms['sex_arm_vectors']:x} (2048-aligned) bss@0x{syms['__bss_start']:x}")
PY

note "build: EL0 payload encodings == assembler output (clang cross-check)"
if command -v clang >/dev/null 2>&1; then
  python3 - <<'PY' || fail "payload word extraction failed"
import struct
words = [0xD2A00829,0xD2800020,0xF9000120,0xD2A8010A,0xF9400141,0xD2800040,
         0xF9000120,0xF9000140,0xD2800060,0xF9000120,0xD2A0082B,0xD61F0160,
         0xD2800080,0xF9000120,0xD2AA000C,0xF9400181,0x14000000]
open('/tmp/gate3_payload_words.bin','wb').write(b''.join(struct.pack('<I',w) for w in words))
PY
  cat > /tmp/gate3_payload.s <<'S'
.text
.global payload
payload:
    movz x9,  #0x41,   lsl #16
    movz x0,  #1
    str  x0,  [x9]
    movz x10, #0x4008, lsl #16
    ldr  x1,  [x10]
    movz x0,  #2
    str  x0,  [x9]
    str  x0,  [x10]
    movz x0,  #3
    str  x0,  [x9]
    movz x11, #0x41,   lsl #16
    br   x11
    movz x0,  #4
    str  x0,  [x9]
    movz x12, #0x5000, lsl #16
    ldr  x1,  [x12]
    b    .
S
  SYSROOT=$(rustc --print sysroot)
  clang -target aarch64-unknown-none -c /tmp/gate3_payload.s -o /tmp/gate3_payload.o 2>/dev/null \
    && "$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy" -O binary --only-section=.text /tmp/gate3_payload.o /tmp/gate3_payload_assembled.bin \
    && cmp -s /tmp/gate3_payload_words.bin /tmp/gate3_payload_assembled.bin \
    && note "payload encodings verified (17/17 words)" \
    || fail "payload encoding mismatch vs assembler"
else
  note "clang absent — payload cross-check SKIPPED (encodings were verified once on 2026-07-21)"
fi

# ── RUNTIME ───────────────────────────────────────────────────────────────
RUNTIME="SKIP_NO_QEMU"
if command -v qemu-system-aarch64 >/dev/null 2>&1; then
  note "runtime: qemu-system-aarch64 found, booting"
  LOG=/tmp/arm64_phase3_qemu.log
  : > "$LOG"
  qemu-system-aarch64 -M virt,gic-version=3 -cpu max -m 512M -nographic \
      -kernel target/aarch64-sex/release/sex-kernel-arm64 > "$LOG" 2>&1 &
  QPID=$!
  sleep 8
  kill "$QPID" 2>/dev/null
  wait "$QPID" 2>/dev/null
  python3 - "$LOG" <<'PY' && RUNTIME="PASS" || fail "runtime marker scan failed"
import sys
try:
    log = open(sys.argv[1], errors='replace').read()
except FileNotFoundError:
    sys.exit(1)
order = ['[sex.arm.entry]', '[sex.arm.el1] ok=1', '[sex.arm.dtb]',
         '[sex.arm.uart] pl011=ok', 'SEX_ARM_ALIVE',
         '[sex.arm.mmu] tables=ok', '[sex.arm.mmu] on=1', '[sex.arm.mmu] wx=1',
         '[sex.arm.mmu] hhdm=ok', 'SEX_ARM_MMU_OK', '[sex.arm.exc] vbar=ok',
         '[sex.arm.el0] enter=1',
         '[sex.arm.neg] el0_read_kernel=aborted',
         '[sex.arm.neg] el0_write_ro=aborted',
         '[sex.arm.neg] el0_exec_data=aborted',
         '[sex.arm.el0] done=1', 'SEX_ARM_PHASE3_OK']
pos = -1
for m in order:
    i = log.find(m, pos + 1)
    if i < 0:
        print(f"[gate3] missing marker: {m}")
        sys.exit(1)
    pos = i
for bad in ['[sex.arm.panic]', 'SEX_ARM_PHASE3_FAIL', 'MISMATCH', 'FATAL',
            'S1POE', 'scheduler', 'userland', 'unexpected=1']:
    if bad in log:
        print(f"[gate3] negative marker present: {bad}")
        sys.exit(1)
# no repeated entry sequence (reboot loop)
if log.count('[sex.arm.entry]') != 1:
    print("[gate3] entry marker repeated — reboot loop")
    sys.exit(1)
print("[gate3] runtime markers OK")
PY
else
  note "runtime: qemu-system-aarch64 absent"
fi

echo "[gate3] X86_PKU_RUNTIME_BASELINE_PENDING=1"
echo "[gate3] RUNTIME=$RUNTIME"
if [ "$FAIL" -ne 0 ]; then echo "[gate3] RESULT=FAIL"; exit 1; fi
echo "[gate3] RESULT=PASS (runtime=$RUNTIME)"
