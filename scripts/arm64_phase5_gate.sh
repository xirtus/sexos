#!/usr/bin/env bash
# ARM64_PHASE_5_PDX_MECHANISM_V1 + ARM64_PHASE_6_DETECT_TAG_V1 gate.
# Exit 0 = PASS (static+build green; runtime PASS or honest SKIP_NO_QEMU).
# Never converts SKIP_NO_QEMU into runtime PASS.
set -uo pipefail
cd "$(dirname "$0")/.."
FAIL=0
note() { echo "[gate56] $*"; }
fail() { echo "[gate56] FAIL: $*"; FAIL=1; }

note "toolchain: $(rustc -Vv | head -1) / $(cargo -V)"

# ── STATIC ────────────────────────────────────────────────────────────────
note "static: no std/libc/POSIX in arch/aarch64"
grep -rEn 'std::|libc|posix|pthread' kernel/src/arch/aarch64/ && fail "std/libc/posix reference found" || true

note "static: no POR/TCR2 register WRITES (detection reads allowed)"
grep -rEin 'msr[[:space:]]+(tcr2_el1|por_el0|por_el1)' kernel/src/arch/aarch64/ \
  | grep -vE ':[0-9]+:[[:space:]]*//' \
  && fail "POR/TCR2 write present (Phase 6 is detection-only)" || true

note "static: no CPACR_EL1 write with any bit set"
grep -rn 'CPACR_EL1' kernel/src/arch/aarch64/ | grep -vE ':[0-9]+:[[:space:]]*//' \
  | grep -v 'xzr' | grep -v 'CPACR_EL1=0' \
  && fail "unexpected CPACR_EL1 manipulation" || true

note "static: ERR_CAP_INVALID literal matches sex-pdx ABI source"
PDXV=$(grep 'pub const ERR_CAP_INVALID' crates/sex-pdx/src/lib.rs | grep -oE '0x[0-9A-Fa-f_]+' | head -1 | tr -d '_')
REGV=$(grep 'pub const ERR_CAP_INVALID' kernel/src/arch/aarch64/registers.rs | grep -oE '0x[0-9A-Fa-f_]+' | head -1 | tr -d '_')
[ "$PDXV" = "$REGV" ] || fail "ERR_CAP_INVALID drift: sex-pdx=$PDXV registers=$REGV"

note "static: ID_AA64MMFR3 decode regression (0x1000000011001111 -> s1poe=0)"
python3 - <<'PY' || fail "ID_AA64MMFR3 decode regression failed"
import re
src = open('kernel/src/arch/aarch64/registers.rs').read()
def shift(name):
    m = re.search(rf'pub const ID_AA64MMFR3_{name}_SHIFT: u64 = (\d+);', src)
    assert m, f"missing {name}_SHIFT"
    return int(m.group(1))
TCRX, SCTLRX, S1PIE, S2PIE, S1POE = (shift('TCRX'), shift('SCTLRX'),
    shift('S1PIE'), shift('S2PIE'), shift('S1POE'))
assert S1POE != 4, "S1POE decoded from bits [7:4] (that nibble is SCTLRX)"
ID = 0x1000000011001111
d = {n: (ID >> s) & 0xF for n, s in
     [('tcrx',TCRX),('sctlrx',SCTLRX),('s1pie',S1PIE),('s2pie',S2PIE),('s1poe',S1POE)]}
want = {'tcrx':1,'sctlrx':1,'s1pie':1,'s2pie':1,'s1poe':0}
assert d == want, f"decode mismatch: {d} != {want}"
# stale-name guard: the misleading TCR2 label must stay gone
assert 'ID_AA64MMFR3_TCR2_SHIFT' not in src, "misleading TCR2 label back"
print(f"[gate56] decode regression ok: {d}")
PY

note "static: arm64-require-poe feature declared"
grep -q 'arm64-require-poe' kernel/Cargo.toml || fail "feature missing from kernel/Cargo.toml"

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

note "static: Phase-5/6 scope pin — files outside the 5/6 changed-set unchanged vs PRE-PHASE5 backup"
B5=$(ls -t /Users/xmacbookm5/Projects/Sex_ArmV9/backups/sex_pre_phase5_*.tar.xz 2>/dev/null | head -1)
if [ -z "$B5" ]; then fail "pre-phase5 backup not found"; else
  TMPD=$(mktemp -d)
  for f in kernel/src/lib.rs kernel/src/main.rs kernel/src/arch.rs \
           kernel/src/arch/aarch64/serial.rs kernel/aarch64-linker.ld \
           aarch64-sex.json; do
    mkdir -p "$TMPD/$(dirname $f)"
    tar -xOf "$B5" "Sex/$f" > "$TMPD/$f" 2>/dev/null || { fail "phase5 backup lacks $f"; continue; }
    cmp -s "$f" "$TMPD/$f" || fail "OUT-OF-SCOPE FILE CHANGED IN PHASE 5/6: $f"
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
scripts/build_arm64_phase2.sh >/tmp/arm64_p56_build.log 2>&1 || fail "aarch64 build failed (see /tmp/arm64_p56_build.log)"

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
        'sex_arm_restore_eret','sex_arm_first','sex_arm_tick','sex_arm_kill_warden',
        'sex_arm_ap_entry','sex_arm_ap_main','arm64_phase4_entry','AP_STACK',
        'sex_arm_svc','arm64_phase6_entry',
        '__kernel_start','__kernel_end','__bss_start']
for s in need:
    assert s in syms, f"missing symbol {s}"
assert syms['sex_arm_vectors'] % 2048 == 0, "vectors not 2048-aligned"
assert syms['__bss_start'] % 4096 == 0, "__bss_start not page-aligned"
u = subprocess.run([f"{sysroot}/lib/rustlib/aarch64-apple-darwin/bin/llvm-nm", '-u',
                    'target/aarch64-sex/release/sex-kernel-arm64'], capture_output=True, text=True)
undef = [l for l in u.stdout.splitlines() if l.strip()]
assert not undef, f"undefined symbols: {undef[:5]}"
print(f"[gate56] ELF ok: entry=0x{e_entry:x} vectors@0x{syms['sex_arm_vectors']:x} bss@0x{syms['__bss_start']:x}")
PY

note "build: EL0 payload encodings == assembler output (clang cross-check)"
if command -v clang >/dev/null 2>&1; then
  SYSROOT=$(rustc --print sysroot)
  OBJCOPY="$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"
  # Phase-3 payload (17 words)
  cat > /tmp/g56_p3.s <<'S'
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
  # Phase-5 payload (18 words)
  cat > /tmp/g56_p5.s <<'S'
.text
.global payload2
payload2:
    movz x9,  #0x41,   lsl #16
    movz x0,  #1
    movz x1,  #41
    svc  #0
    str  x0,  [x9, #16]
    movz x0,  #2
    movz x1,  #0xDEAD
    svc  #0
    str  x0,  [x9, #24]
    movz x0,  #2
    movz x1,  #42
    svc  #0
    str  x0,  [x9, #32]
    movz x0,  #5
    str  x0,  [x9]
    movz x12, #0x6000, lsl #16
    ldr  x1,  [x12]
    b    .
S
  python3 - <<'PY' || fail "payload word write failed"
import struct
p3 = [0xD2A00829,0xD2800020,0xF9000120,0xD2A8010A,0xF9400141,0xD2800040,
      0xF9000120,0xF9000140,0xD2800060,0xF9000120,0xD2A0082B,0xD61F0160,
      0xD2800080,0xF9000120,0xD2AA000C,0xF9400181,0x14000000]
p5 = [0xD2A00829,0xD2800020,0xD2800521,0xD4000001,0xF9000920,0xD2800040,
      0xD29BD5A1,0xD4000001,0xF9000D20,0xD2800040,0xD2800541,0xD4000001,
      0xF9001120,0xD28000A0,0xF9000120,0xD2AC000C,0xF9400181,0x14000000]
open('/tmp/g56_p3.bin','wb').write(b''.join(struct.pack('<I',w) for w in p3))
open('/tmp/g56_p5.bin','wb').write(b''.join(struct.pack('<I',w) for w in p5))
PY
  clang -target aarch64-unknown-none -c /tmp/g56_p3.s -o /tmp/g56_p3.o 2>/dev/null \
    && "$OBJCOPY" -O binary --only-section=.text /tmp/g56_p3.o /tmp/g56_p3_asm.bin \
    && cmp -s /tmp/g56_p3.bin /tmp/g56_p3_asm.bin \
    && note "phase3 payload verified (17/17)" \
    || fail "phase3 payload encoding mismatch"
  clang -target aarch64-unknown-none -c /tmp/g56_p5.s -o /tmp/g56_p5.o 2>/dev/null \
    && "$OBJCOPY" -O binary --only-section=.text /tmp/g56_p5.o /tmp/g56_p5_asm.bin \
    && cmp -s /tmp/g56_p5.bin /tmp/g56_p5_asm.bin \
    && note "phase5 payload verified (18/18)" \
    || fail "phase5 payload encoding mismatch"
else
  note "clang absent — payload cross-check SKIPPED (verified 2026-07-21/22)"
fi

note "static: tag-3 code-continuation literal present (USER_CODE_VA+0x30)"
grep -q '=0x400030' kernel/src/arch/aarch64/exceptions.rs || fail "missing 0x400030 continuation literal"

# ── RUNTIME ───────────────────────────────────────────────────────────────
RUNTIME="SKIP_NO_QEMU"
if command -v qemu-system-aarch64 >/dev/null 2>&1; then
  note "runtime: qemu-system-aarch64 found, booting"
  LOG=/tmp/arm64_phase56_qemu.log
  : > "$LOG"
  qemu-system-aarch64 -M virt,gic-version=3,virtualization=on -cpu max -smp 2 -m 512M -nographic \
      -kernel target/aarch64-sex/release/sex-kernel-arm64 > "$LOG" 2>&1 &
  QPID=$!
  sleep 8
  kill "$QPID" 2>/dev/null
  wait "$QPID" 2>/dev/null
  python3 - "$LOG" <<'PY' && RUNTIME="PASS" || { RUNTIME="FAIL"; fail "runtime marker scan failed"; }
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
         '[sex.arm.el0] done=1', 'SEX_ARM_PHASE3_OK',
         '[sex.arm.p4] entry=1', '[sex.arm.gic] gicd=ok', '[sex.arm.gic] gicr=ok',
         '[sex.arm.gic] cpuif=ok', '[sex.arm.gic] ppi30=on',
         '[sex.arm.smp] ap_boot=1', 'SEX_ARM_AP_OK',
         '[sex.arm.smp] ap_ack=1', '[sex.arm.timer] freq=',
         '[sex.arm.sched] start=1',
         # Runtime truth (2026-07-22): each task burns its 3 prints inside
         # one 10ms tick slice (prints at CPU speed, switches at 100Hz), so
         # the canonical order is a1,a2,a3,b1,b2,b3 — the preemption switch,
         # kill containment and survivor completion are what is proven.
         '[sex.arm.sched] a_run=1', '[sex.arm.sched] a_run=2',
         '[sex.arm.sched] a_run=3', '[sex.arm.sched] b_run=1',
         '[sex.arm.sched] b_run=2', '[sex.arm.sched] b_run=3',
         '[sex.arm.neg] kill_task=contained',
         '[sex.arm.sched] a_after_dead=10', 'SEX_ARM_PHASE4_OK',
         '[sex.arm.p5] entry=1', '[sex.arm.timer] off=1',
         '[sex.arm.el0] enter2=1', '[sex.arm.el0] done2=1',
         '[sex.arm.svc] echo=ok', '[sex.arm.svc] cap_bad=ERR_CAP_INVALID',
         '[sex.arm.svc] cap_good=ok', 'SEX_ARM_PHASE5_OK',
         '[sex.arm.p6] entry=1', '[sex.arm.poe] id_aa64mmfr3=',
         'SEX_ARM_POE_TAGGED', 'SEX_ARM_PHASE6_OK',
         '[sex.arm.p7] deferred=1']
pos = -1
for m in order:
    i = log.find(m, pos + 1)
    if i < 0:
        print(f"[gate56] missing marker: {m}")
        sys.exit(1)
    pos = i
for bad in ['[sex.arm.panic]', 'SEX_ARM_PHASE3_FAIL', 'SEX_ARM_PHASE4_FAIL',
            'SEX_ARM_PHASE5_FAIL', 'FAILCLOSED', 'MISMATCH', 'FATAL',
            'unexpected=1', 'stray_irq', 'kill_survivor', 'ap_timeout',
            'psci_ret']:
    if bad in log:
        print(f"[gate56] negative marker present: {bad}")
        sys.exit(1)
if log.count('[sex.arm.entry]') != 1:
    print("[gate56] entry marker repeated — reboot loop")
    sys.exit(1)
print("[gate56] runtime markers OK")
PY
else
  note "runtime: qemu-system-aarch64 absent"
fi

echo "[gate56] X86_PKU_RUNTIME_BASELINE_PENDING=1"
echo "[gate56] RUNTIME=$RUNTIME"
if [ "$FAIL" -ne 0 ]; then echo "[gate56] RESULT=FAIL"; exit 1; fi
echo "[gate56] RESULT=PASS (runtime=$RUNTIME)"
