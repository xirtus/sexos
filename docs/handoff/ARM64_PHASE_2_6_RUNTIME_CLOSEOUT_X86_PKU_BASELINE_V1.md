# ARM64 PHASE 2-6 RUNTIME CLOSEOUT + X86 PKU BASELINE V1 (2026-07-22)

Mission: ARM64_PHASE_2_6_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1.
Executed on THIS host after provisioning (QEMU 11.0.2 both archs,
xorriso, ripgrep, limine v7). Nested-session attempt was sandboxed;
all execution + evidence produced by the permissioned parent session.
SEE ALSO: docs/handoff/ARM64_RUNTIME_FIXES_V1.md (6 kernel bugs +
3 gate bugs found BY runtime and fixed during Lane A).

## A. OVERALL VERDICT
- Lane A: ARM_PHASE_2_6_RUNTIME_PASS
- Lane B: X86_PKU_RUNTIME_PASS (mission-enumerated proofs; canonical
  gate still RED on PRE-EXISTING ORDER_GATE — see F)
- P1: CASE B — S1POE REPORTED -> Phase 6B probes mission UNBLOCKED
- FINAL: ARM64_PHASE_6B_PROBES_UNBLOCKED

## B. HOST / TOOLCHAIN / QEMU
- Host: Darwin aarch64-apple-darwin (macOS 26.x, T8142)
- rustc 1.99.0-nightly (0e29c21d9 2026-07-21) / cargo 3efb1f477 [pinned]
- qemu-system-aarch64 11.0.2, qemu-system-x86_64 11.0.2 (Homebrew, TCG)
- ripgrep 14.1.1, xorriso 1.5.8.pl02, limine v7.x-binary (native build)
- Env fix: root x86_64-sex.json rustc-abi x86-softfloat -> softfloat
  (upstream rename, same ABI; kernel spec untouched)

## C. ARM PHASE 2-6 ORDERED PROOF (all OBSERVED, single boot,
      logs/arm64_phase2_6_runtime_closeout.log)
- P2: current_el=2 (virtualization=on), EL2->EL1 ok=1,
  dtb x0=0 present=0 note=elf_boot_no_fdt (QEMU ELF boot loads NO FDT —
  proven by full 512MiB pmemsave scan; platform addresses are fixed),
  pl011=ok, SEX_ARM_ALIVE
- P3: tables=ok on=1 wx=1 hhdm=ok SEX_ARM_MMU_OK vbar=ok enter=1,
  el0_read_kernel/write_ro/exec_data=aborted, done=1, SEX_ARM_PHASE3_OK
- P4: gicd/gicr/cpuif=ok ppi30=on, ap_boot=1 SEX_ARM_AP_OK ap_ack=1,
  freq=1000000000 hz=100, start=1, a_run 1-3 + b_run 1-3,
  kill_task=contained far=0x0, a_after_dead=10, SEX_ARM_PHASE4_OK
- P5: timer off=1, enter2/done2, echo=ok, cap_bad=ERR_CAP_INVALID
  (exact 0xFFFF_FFFF_FFFF_FFFC), cap_good=ok, SEX_ARM_PHASE5_OK
- P6: id_aa64mmfr3=0x1000000011001111, SEX_ARM_POE_TAGGED
  poindex_default=0 nonzero_tags=0, SEX_ARM_PHASE6_OK,
  [sex.arm.p7] deferred=1 reason=runtime_probes_required
- Gate: scripts/arm64_phase5_gate.sh RESULT=PASS (runtime=PASS) on
  clean tree (x86 build GREEN 153 warnings pinned; aarch64 build GREEN;
  payload cross-checks 17/17 + 18/18)

## D. ARM FAULT SCAN
Zero: no panic/unexpected-abort/recursion/storm/stall/corruption/
reboot/host-crash. Negative markers absent. No S1POE-support CLAIM
made (status=UNPROVEN isolation=none printed by the kernel itself).

## E. S1PIE/S1POE P1 RESULT  *** CORRECTED 2026-07-22 (see ARM64_PHASE_6B handoff) ***
- Raw ID_AA64MMFR3_EL1 = 0x1000000011001111 (QEMU 11.0.2 -cpu max)
- The provisional decode printed by the kernel used WRONG field
  positions (S1POE was read from [7:4]; the correct position is
  [19:16] = S1POE, which is 0). CORRECTED DECODE: S1POE=0 ABSENT;
  S1PIE=1, S2PIE=1, TCRX=1, SCTLRX=1 present-but-unimplemented.
- THE "S1POE REPORTED" STATEMENT IN THIS SECTION'S ORIGINAL TEXT IS
  REVOKED. QEMU 11.0.2 and master implement no S1POE (no POR/TCR2_EL1
  sysregs, no POE in the translation walk). The UNVERIFIED marking on
  the field positions is what caught this — verification did its job.
- Phase 6B: NOT unblocked (no platform). Phase 7: BLOCKED.
  Original (revoked) text preserved below for provenance:
- Raw ID_AA64MMFR3_EL1 = 0x1000000011001111 (QEMU 11.0.2 -cpu max)
- Provisional decode (source-marked UNVERIFIED positions):
  S1POE[7:4]=1, S1PIE[11:8]=1, TCR2[15:12]=1 -> all REPORTED
- Interpretation confidence: feature-bit positions plausible but
  UNVERIFIED against the ARM ARM; functional behavior UNPROVEN.
- Zero production POR/TCR2/CPACR writes (gate-enforced, re-verified).
- Phase 6B: UNBLOCKED as a separate bounded-probes mission
  (ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1). Phase 7 stays BLOCKED
  until P2-P6 pass. CPACR_EL1.E0POE=0 forever rule unchanged.

## F. X86 PKU RUNTIME PROOF (logs/x86_pku_runtime_baseline.log)
- CR4.PKE enabled at runtime; PDs spawn with PKU keys and run
  (87 task.running); LAPIC clock calibrated+advancing; FAULT/SPAWN/
  CLOCK/SCHED/BOOTGRAPH/CAP_GRANT/SEXFILES PASS; zero #UD/#PF/#GP.
- ORDER_GATE FAIL — PRE-EXISTING (same receiver_ready>=send inversion
  as the last Linux gate-host run). Not modified, not reinterpreted,
  not owned by the Arm port.
- Env fixes to build on this host: ripgrep, limine v7 native,
  xorriso, root target-json rustc-abi rename. All recorded in the log.

## G. FILES CHANGED
- Kernel (runtime-found fixes, see ARM64_RUNTIME_FIXES_V1.md):
  exceptions.rs, early.rs, gicv3.rs, smp.rs, phase5.rs (debug add+remove)
- scripts/arm64_phase5_gate.sh (QEMU cmd: virtualization=on -smp 2;
  marker order vs runtime truth; RUNTIME=FAIL honesty; 0x400030 pin)
- x86_64-sex.json (rustc-abi rename)
- logs/arm64_phase2_6_runtime_closeout.log, logs/x86_pku_runtime_baseline.log
- docs/handoff/* (this file, RUNTIME_FIXES, CONVERSION_STATUS)
- NO changes to pinned x86 files (gate re-verified byte-identical)

## H. NEXT STATUS
Phases 2-5: runtime CLOSED on QEMU. 5B (PDX parity): unproven by design
(userland phase). 6A: runtime PASS. 6B: UNBLOCKED — author
ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1 (bounded P2-P6 probe code; note:
the phase5 gate's no-POR/TCR2-write static check must be amended for a
designated probe module only). 7: BLOCKED until probes pass.
8/9: BLOCKED (runtime/hardware). POIndex assignments remain PROVISIONAL.
E0POE=0 forever. X86_PKU_RUNTIME_BASELINE: PKU-runtime REPRODUCED here;
ORDER_GATE RED remains the pre-existing x86 debt.

VERDICT: ARM64_PHASE_6B_PROBES_UNBLOCKED
