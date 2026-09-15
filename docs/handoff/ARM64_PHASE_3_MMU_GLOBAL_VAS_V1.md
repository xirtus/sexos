# ARM64_PHASE_3_MMU_GLOBAL_VAS_V1 — Handoff (2026-07-21)
(plan alias: ARM_PHASE3_MMU_V1)

## Result
STATIC+BUILD: **PASS**. RUNTIME: **SKIP_NO_QEMU** (qemu-system-aarch64
absent; never converted to PASS). x86 unchanged and GREEN.
X86_PKU_RUNTIME_BASELINE_PENDING=1 (still owed; closeout prompt unchanged).

## Backup
- Sex_ArmV9/backups/sex_pre_phase3_20260721_234142.tar.xz
- SHA-256: d4ab0b10379edd1cdde2b67593bb6391737bf4c36ebd8b9cd412ab5ef72df782
- Doc backup: Sex_ArmV9/backups/doc_pre_closeout_20260721/

## Scope + deviations (read before trusting)
Plan Phase 3 allows: arch/aarch64/mmu.rs, elf.rs arch-backend hooks.
Forbidden: capability.rs, ipc.rs (gate-verified byte-identical vs
pre-phase2 backup, along with the full Phase-2 forbidden set).
DEVIATION 1 — minimal exceptions.rs (VBAR + sync warden): the plan assigns
exceptions.rs to Phase 4, but Phase 3's own negative gates (EL0 read of
kernel VA / EL0 write to RO text must sync-abort) are untestable without
a sync warden. Implemented the SMALLEST enabler: all 16 vector slots are
halt stubs except sync-EL1h (print+halt) and sync-lower-A64 (tag-checked
expected-fault warden). Phase 4 replaces/expands this.
DEVIATION 2 — no real sexsh ELF: no aarch64 userland backend exists
(plan table B: userland parity is a later USERLAND PHASE) and the raw
-kernel dev path has no initrd/module loader. Instead a hand-encoded
17-word A64 payload (el0_smoke.rs, every word documented + verified
17/17 against clang -target aarch64-unknown-none assembler output)
proves the SAME gate facts: EL0 execution, EL0-read-of-kernel abort,
EL0-write-to-RO-text abort, EL0-exec-from-RW-data abort. Real ELF
loading (sexsh via module path) remains outstanding.

## Design (single global VAS — Sex SAS model, mirrors x86 single CR3)
- TTBR0_EL1 (lower 48-bit): kernel image identity map (text/rodata RO+X
  EL1-only below __bss_start; data/bss/stack RW+NX EL1-only), MMIO
  identity block (2 MiB @ 0x0900_0000, Device-nGnRnE, EL1-only, XN —
  covers PL011), user smoke pages. ALL future user PDs go here — the
  TTBR is NEVER per-PD switched (plan STOP FIRST "per-PD TTBR needed"
  NOT triggered). Per-PD isolation = Phase 6/7 POE, not page tables.
- TTBR1_EL1 (upper 48-bit): HHDM @ 0xFFFF_8000_0000_0000 — 1 GiB Device
  block (MMIO alias: GIC/UART/virtio) + 1 GiB Normal block (DRAM
  0x40000000-0x7FFFFFFF, NX). Platform-fixed to QEMU virt; DTB memory
  parsing is Phase 4+.
- TCR_EL1=0x2_B5103510 (48-bit both, 4K granule both, WB WA, inner SH,
  IPS=40-bit), MAIR=0xFF00 (idx0 Device-nGnRnE, idx1 Normal WB),
  SCTLR=0x30D8180D (SAFE|M|C|I|WXN|SA). WXN readback-verified.
- Page tables: static pool of 16 × 4 KiB tables in .bss (bump alloc at
  boot, no heap). 4 KiB pages for kernel/user; 1 GiB blocks for HHDM;
  2 MiB block for MMIO window. All attrs: AF set, SH inner (Normal).
- W^X: per-descriptor UXN/PXN + SCTLR.WXN. Kernel text RO+X (AP=RO-EL1),
  kernel data NX, user code RO+UXN-clear (EL0 X), user data UXN+PXN.

## Files changed (complete)
NEW: kernel/src/arch/aarch64/{mmu.rs, exceptions.rs, el0_smoke.rs},
scripts/arm64_phase3_gate.sh,
docs/handoff/ARM64_PHASE_3_MMU_GLOBAL_VAS_V1.md
EDIT: kernel/src/arch/aarch64/mod.rs (+3 mods, doc update),
kernel/src/arch/aarch64/early.rs (Phase-3 sequence after SEX_ARM_ALIVE),
kernel/src/arch/aarch64/registers.rs (Phase-3 constants appended).
UNCHANGED (gate scope-pin vs pre-phase3 backup): lib.rs, src/main.rs,
arch.rs, kernel/Cargo.toml, arch/aarch64/{main.rs, serial.rs},
aarch64-linker.ld, aarch64-sex.json.

## Expected runtime markers (ordered; gate enforces order when QEMU runs)
[sex.arm.entry] → [sex.arm.el1] ok=1 → [sex.arm.dtb] → [sex.arm.uart]
→ SEX_ARM_ALIVE → [sex.arm.mmu] tables=ok → on=1 → wx=1 → hhdm=ok →
SEX_ARM_MMU_OK → [sex.arm.exc] vbar=ok → [sex.arm.el0] enter=1 →
[sex.arm.neg] el0_read_kernel=aborted → el0_write_ro=aborted →
el0_exec_data=aborted → [sex.arm.el0] done=1 → SEX_ARM_PHASE3_OK.
Negative scan: [sex.arm.panic], MISMATCH, FATAL, SEX_ARM_PHASE3_FAIL,
repeated entry marker (reboot loop), any S1POE claim.

## Warden protocol (EL0 sync)
Payload writes tag (1..4) to mailbox[0] before each deliberate fault.
Warden checks tag + ESR.EC + FAR: tag1/2 → EC=0x24 (data abort lower),
FAR=0x40080000; tag3 → EC=0x20 (insn abort lower), FAR=0x410000;
tag4 → EC=0x24, FAR=0x50000000 → prints done + SEX_ARM_PHASE3_OK, halt.
Match → ELR+=4, ERET. Mismatch/unexpected → detail + SEX_ARM_PHASE3_FAIL,
halt. Warden saves/restores x0-x15+x30 (payload keeps live values in
x9-x12 across faults — Rust clobbers caller-saved regs).

## Static proofs this phase (no runtime claims)
- ELF: machine=183, entry=0x40080000, 0 undefined symbols,
  sex_arm_vectors @0x40081000 (2048-aligned), __bss_start @0x40085000
  (page-aligned → RO+X/RW+NX split safe).
- Payload: 17/17 words identical to clang assembler output for the
  documented instructions (gate re-verifies via clang when present).
- Table-index math: kernel L0[0]→L1[1]→L2[0]→L3[128..]; user
  L0[0]→L1[0]→L2[2]→L3; MMIO L1[0]→L2[0x48] block; HHDM L0[511]→L1[0/1]
  blocks. No descriptor collisions (walked through by hand + code).
- Disassembly cannot prove: MMU enable actually works, HHDM reads, EL0
  entry, warden correctness, absence of reboot loop. ALL runtime claims
  deferred to the closeout mission (unchanged prompt).

## Build results
- x86 (cd kernel && cargo build): GREEN, exactly 153 warnings (pinned
  toolchain rustc 1.99.0-nightly 0e29c21d9 / cargo 3efb1f477).
- aarch64: GREEN. Image target/aarch64-sex/release/sex-kernel-arm64,
  __kernel_end=0x400a8010 (grew: 16×4 KiB table pool + 3 user pages).

## Gotchas added
11. `static mut` pool needs `#[derive(Clone,Copy)]` for array-repeat
    init (or a const EMPTY pattern) — E0277 otherwise.
12. Hex literals: `X` is not a hex digit (0x5EX5… typo cost one build).
13. Payload const arrays get constant-propagated by LLVM into immediate
    stores in prepare() — the byte sequence does NOT appear verbatim in
    the image (same class as marker unrolling). Verify by assembler
    cross-check, not by searching image bytes.
14. llvm-objdump in the rust sysroot has no `-b binary`; to disassemble
    raw words use clang -target aarch64-unknown-none to ASSEMBLE the
    reference and llvm-objcopy -O binary to extract, then cmp.

## Next safe phase
ARM64_PHASE_4_GIC_TIMER_SCHED_V1 (plan Phase 4: exceptions expansion,
GICv3, CNTP timer, context switch, 2-task ping-pong, PSCI AP bring-up).
Still blocked for RUNTIME proof on the closeout mission; static/build
work may proceed per this phase's precedent (code + gate + honest SKIP).
