# ARM64_PHASE_2_ENTRY_SERIAL_V1 — Handoff (2026-07-21)

## Result
STATIC+BUILD: **PASS**. RUNTIME: **SKIP_NO_QEMU** (qemu-system-aarch64
absent on this host; never converted to PASS).
X86_PKU_RUNTIME_BASELINE_PENDING=1 (unchanged debt from Phase 0/1).

## Toolchain (pinned context — recorded 2026-07-21)
- rustc 1.99.0-nightly (0e29c21d9 2026-07-21), host aarch64-apple-darwin,
  LLVM 22.1.8 (`rustc -Vv` full: binary=rustc,
  commit-hash=0e29c21d93abc9bdc330182be1fbd33cfd5fee68,
  commit-date=2026-07-21, release=1.99.0-nightly)
- cargo 1.99.0-nightly (3efb1f477 2026-07-17)
- WARNING: the gate's exact "153 warnings" pin for the x86 build is only
  meaningful under THIS toolchain. Any rustc/cargo change invalidates the
  pin — record the new toolchain and re-baseline the count before
  comparing. Do not treat warning-count drift across toolchains as a
  regression signal without recording the toolchain difference first.

## Backup
- Path: Sex_ArmV9/backups/sex_pre_phase2_20260721_220749.tar.xz
- SHA-256: 0fb670bf1b815ad9e51ed3099038c651061aaf9fa2f28991bcf944b788bfdce0
- 3296 files, ~22 MiB. Verified contains lib.rs, arch.rs, pku.rs,
  interrupts.rs, kernel/linker.ld. Prior baseline archive NOT nested
  (backups live outside the Sex tree).

## Files changed (complete)
NEW:
- aarch64-sex.json (repo root; custom target, NOT default)
- kernel/aarch64-linker.ld
- kernel/src/arch/aarch64/{mod,registers,serial,early,main}.rs
- scripts/build_arm64_phase2.sh, run_arm64_phase2.sh, arm64_phase2_gate.sh
EDITED:
- kernel/src/arch.rs — added aarch64 cfg arm (x86 arm untouched)
- kernel/src/lib.rs — cfg(target_arch="x86_64") on all x86-only modules,
  limine statics, allocator, kernel_init, bootstrap_advanced_services;
  arch + protection stay common; mem-fns stay common (portable)
- kernel/Cargo.toml — all deps moved to
  [target.'cfg(target_arch = "x86_64")'.dependencies]; added
  [features] arm64 = []; added [[bin]] sex-kernel-arm64
  (required-features=["arm64"], path src/arch/aarch64/main.rs)
UNTOUCHED (gate-verified vs backup): pku.rs, interrupts.rs, scheduler.rs,
ipc.rs, ipc_ring.rs, capability.rs, gdt.rs, apic.rs, smp.rs,
kernel/linker.ld, crates/sex-pdx/**. x86 src/main.rs untouched.

## Boot path
- Reset EL: EL2 expected (QEMU virt boots kernels at EL2 with
  virtualization=on). Entry asm preserves x0→x20 (DTB), captures
  CurrentEL→x21.
- EL2→EL1: HCR_EL2.RW=1; SCTLR_EL1=0x30D00800 (MMU/caches off, RES1);
  CPACR_EL1=0 (FP/SIMD trapped; softfloat kernel); SPSR_EL2=0x3C5
  (EL1h, DAIF masked); ELR_EL2=continuation; SP_EL1=__boot_stack_top;
  ERET. EL1-direct entry also handled. EL0/EL3 → 'E' on PL011 + WFE.
- Stack: 64 KiB in .bss, __boot_stack_bottom=0x40082000,
  __boot_stack_top=0x40092000 (symbol-verified).
- BSS: cleared in asm (spans stack; no live data yet). __bss_start=
  0x40082000, __bss_end=0x40092000.
- Rust entry: arm64_early_main(dtb=x20, entered_el=x21) — no alloc,
  no locks, no fmt heap, no kernel_init(), no Limine.
- UART: PL011 @ 0x0900_0000 (platform-fixed to QEMU virt, documented).
  No divisor init (QEMU accepts data without clock setup). TXFF poll
  then DR write; volatile only; NO per-char DMB/DSB (MMU off ⇒
  Device-nGnRnE; same-peripheral accesses ordered) — rationale in file.
  *** PROVISIONAL until runtime succeeds ***: the MMU-off ordering
  justification is architectural reasoning, not observed fact.
  Disassembly cannot prove UART delivery, EL2→EL1 transition, DTB
  validity, or absence of a reboot loop. If the runtime closeout
  (ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1) shows missing or
  garbled output, re-add DMB SY (or DSB) after each DR write FIRST
  before suspecting deeper bugs — this is the cheapest known-unknown.
- Halt: bounded WFE loop. Panic: [sex.arm.panic] + WFE (AArch64 bin's
  own handler; x86 handler untouched).

## Addresses
- Link/load base: 0x40080000 (QEMU virt -kernel convention; VMA==LMA,
  physical identity, no higher-half yet).
- ELF: e_machine=183 (EM_AARCH64), e_type=2 (EXEC), e_entry=0x40080000
  (inside first PT_LOAD [0x40080000, 0x40080f38)). Verified via gate.
- Undefined symbols: 0 (llvm-nm -u).

## Build results
- x86 (cd kernel && cargo build): GREEN, exactly 153 warnings
  (gate-enforced), 0 errors. Unchanged from baseline.
  (Pin valid ONLY under the toolchain recorded above; see Toolchain.)
- AArch64 (scripts/build_arm64_phase2.sh): GREEN, 4 warnings
  (pre-existing mem-fn runtime-symbol lints; code untouched per rules).
  Image: target/aarch64-sex/release/sex-kernel-arm64 (73 KiB).

## QEMU command (for a capable host)
  qemu-system-aarch64 -M virt,gic-version=3 -cpu max -m 512M \
    -nographic -kernel target/aarch64-sex/release/sex-kernel-arm64
Expected ordered markers (verified present in the IMAGE as code; see
gotcha #2): [sex.arm.entry] current_el=2 → [sex.arm.el1] ok=1 →
[sex.arm.dtb] ptr=0x4… → [sex.arm.dtb] magic=ok →
[sex.arm.uart] pl011=ok → SEX_ARM_ALIVE.

## Recurring gotchas (save future agents)
1. `rg` IS A SHELL FUNCTION on this machine (wraps the claude binary),
   NOT a binary — scripts must use grep -rn. First gate run silently
   skipped 3 static checks because of this. Fixed; gate now rg-free.
2. Release LLVM UNROLLS short write_str calls into immediate byte
   stores: "SEX_ARM_ALIVE", "ok=1", "[sex.arm.panic]" never appear in
   rodata. Disassembly proved byte-immediate stores with TXFF polling
   (mov w9,#0x53='S', str →DR …). Marker verification MUST be
   runtime-based, never string-presence-in-binary-based.
3. core::hint::spin_loop() on aarch64 emits ISB (this nightly).
4. global_asm cannot reference Rust constants — SCTLR/SPSR literals in
   entry asm mirror registers::SCTLR_EL1_SAFE/SPSR_EL1H_DAIF/HCR_EL2_RW
   by comment contract; keep them in sync.
5. build_payload.sh has a STALE absolute TARGET path
   (/home/xirtus_arch/x86_64-sex.json) — canonical x86 build on this
   machine is `cd kernel && cargo build`; the Arm script uses the
   repo-local aarch64-sex.json and mirrors payload -Z flags.
6. Repo root .cargo/config.toml lacks build.target — always pass
   --target explicitly from root, or build from kernel/.
7. Workspace root builds ALL members; always scope -p sex-kernel
   --bin sex-kernel-arm64 --features sex-kernel/arm64 for Arm images.
8. Target-specific deps ([target.'cfg(target_arch)'.dependencies])
   evaluate "arch" from custom JSON — works, x86 resolution unchanged.
9. rust-lld emits PT_LOAD p_paddr == vaddr for static links; QEMU
   -kernel on aarch64 loads ELF segments at p_paddr and jumps to
   e_entry — no objcopy/raw Image step needed.
10. DTB magic is BIG-endian (0xD00DFEED); read volatile u32 then
    u32::from_be. QEMU places DTB at start of DRAM; x0 at reset.

## Runtime faults observed
None — no runtime executed (SKIP_NO_QEMU). No host segfaults.

## Next safe phase
ARM64_PHASE_3_MMU_GLOBAL_VAS_V1 — gated on the x86 PKU-on runtime debt
(option A: -cpu max,+pku gate passes on a QEMU-capable host; option B:
explicit STOP FIRST authorization). Phase 2 proceeded because its
changes are cfg-isolated and additive.

## Runtime closeout attempts
- 2026-07-21 (this host, aarch64-apple-darwin, macOS 26.5.2):
  ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1 executed.
  Lane A: build GREEN (ELF machine=183, entry=0x40080000, 0 undefined
  symbols, no source changes); runtime ARM_RUNTIME_SKIP_NO_QEMU
  (runner exit 77, qemu-system-aarch64 absent).
  Lane B: X86_PKU_RUNTIME_SKIP_NO_QEMU (qemu-system-x86_64 absent;
  no CPU-model substitution attempted; sexos-v1.0.0.iso absent here —
  canonical gate host is Linux).
  Verdict: BLOCKED — Phase 3 remains blocked.
  Logs: logs/arm64_phase2_runtime_closeout.log,
  logs/x86_pku_runtime_baseline.log.
  Full record: docs/handoff/ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1.md
