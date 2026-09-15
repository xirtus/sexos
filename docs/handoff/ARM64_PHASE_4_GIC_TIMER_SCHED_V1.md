# ARM64_PHASE_4_GIC_TIMER_SCHED_V1 — Handoff (2026-07-21/22)
(plan alias: ARM_PHASE4_SCHED_V1)

## Result
STATIC+BUILD: **PASS**. RUNTIME: **SKIP_NO_QEMU** (never converted).
x86 unchanged and GREEN (153 warnings, pinned toolchain).
X86_PKU_RUNTIME_BASELINE_PENDING=1 still owed.

## Backup
- Sex_ArmV9/backups/sex_pre_phase4_20260722_001220.tar.xz
- SHA-256 (first 16): 08172d6f563275061c137bbb8966bd3d…

## Scope + deviations
Plan Phase 4 allows: arch/aarch64/{exceptions,context,gicv3,timer,smp,
core_local}.rs + minimal scheduler.rs cfg hooks. Forbidden: scheduler
policy/TaskState changes (x86 scheduler.rs gate-verified UNTOUCHED vs
pre-phase2 backup).
DEVIATION 1 — scheduler is arch-local sched.rs, NOT the x86 scheduler:
x86 scheduler.rs is cfg(target_arch="x86_64")-gated at lib.rs and cannot
compile for aarch64; porting its policy is a later phase. This phase
proves the MECHANISM the gate asks for: 2-task preemptive ping-pong on
1 core driven by CNTP tick IRQ, full-frame save/restore via ERET, plus
kill-task containment. core_local.rs folded into a TPIDR_EL1 write
(phase4.rs/smp.rs) — no separate module needed at this size.
DEVIATION 2 — kill-task negative uses a KERNEL task's deliberate null
write (contained via EL1h sync warden → survivor resumes). True EL0-task
kill containment needs real userland (later phase); the mechanism
(sync warden → drop task → resume survivor) is what x86
USER_FAULT_CONTAINMENT does.

## Design
- GICv3 (gicv3.rs): QEMU virt GICD@0x08000000 + GICR@0x080A0000 via
  HHDM device alias (NOT identity-mapped). GICD_CTLR = ARE_NS|Grp1NS;
  GICR_WAKER cleared + ChildrenAsleep poll (bounded 1e6); ICC_SRE_EL1,
  PMR=0xFF, IGRPEN1=1; PPI 30 priority 0x80 + ISENABLER0. Readback-
  verified where meaningful.
- Timer (timer.rs): CNTFRQ_EL0 read (FATAL if 0), CNTP_TVAL_EL0 =
  freq/100 reloaded each IRQ, CNTP_CTL_EL0 ENABLE|IMASK=0. Timer init
  is LAST in phase4.rs (after SMP) so no pre-scheduler expiry can make
  the a/b interleave nondeterministic.
- Context (context.rs): 272-byte frame [x0-x29, x30, ELR, SPSR] built by
  handlers; shared restore tail sex_arm_restore_eret; initial frames
  synthesized identically (SPSR=0x345 EL1h IRQ-unmasked). Per-arch
  layout — NOT unified with x86 TaskContext (audit rule).
- Sched (sched.rs): tick → save current frame → alternate A/B → restore.
  Task A bounded prints; task B faults (null write) on run 3 →
  sex_arm_kill_warden (started? survivor-check) → B_DEAD → A resumes and
  counts 10 activations → SEX_ARM_PHASE4_OK. A-fault = fatal (kill_survivor).
- SMP (smp.rs): PSCI CPU_ON (HVC, 0xC4000003, mpidr=1, entry=physical
  ap_entry). BSP cleans+invalidates D-cache over the kernel image +
  IC IALLU first (AP reads TTBR addrs with caches OFF — coherence
  protocol documented in-file). AP: EL2→EL1 drop mirrors BSP entry,
  own 16 KiB stack, MMU on with SHARED tables, VBAR, TPIDR=1,
  SEX_ARM_AP_OK, AP_DONE (post-MMU coherent), WFE. BSP ack + timeout
  1e8 spins = FATAL.
- exceptions.rs: slot 0x280 = IRQ handler (frame save, ICC_IAR1 read,
  spurious-1023 check, tick, ICC_EOIR1, shared restore); slot 0x200 =
  kill warden (frame save → sex_arm_kill_warden → restore-or-halt);
  Phase-3 EL0 warden now returns 2 on final tag → asm chains via ERET
  to arm64_phase4_entry (EL1h, DAIF masked).
- main.rs: CNTHCTL_EL2=3 in the BSP EL2 drop (EL1 CNTP access; reset
  value otherwise impl-def). Mirrored in the AP EL2 path.

## Files changed
NEW: arch/aarch64/{gicv3.rs, timer.rs, context.rs, sched.rs, smp.rs,
phase4.rs}, scripts/arm64_phase4_gate.sh, this handoff.
EDIT: exceptions.rs (IRQ + kill warden + p4 chain), main.rs (CNTHCTL),
mmu.rs (TTBR0/1_ADDR publish), registers.rs (phase-4 consts),
mod.rs (+6 mods), phase3 gate untouched.
Scope-pinned unchanged vs pre-phase4 backup (gate): lib.rs, src/main.rs,
arch.rs, Cargo.toml, aarch64-linker.ld, aarch64-sex.json, serial.rs,
early.rs. x86 forbidden set byte-identical vs pre-phase2 backup.

## Expected runtime markers (ordered; gate enforces)
… Phase 2/3 sequence … SEX_ARM_PHASE3_OK → [sex.arm.p4] entry=1 →
[sex.arm.gic] gicd=ok → gicr=ok → cpuif=ok → ppi30=on →
[sex.arm.timer] freq=<n> hz=100 → [sex.arm.smp] ap_boot=1 →
SEX_ARM_AP_OK → [sex.arm.smp] ap_ack=1 → [sex.arm.sched] start=1 →
a_run=1 → b_run=1 → a_run=2 → b_run=2 → a_run=3 → b_run=3 →
[sex.arm.neg] kill_task=contained → a_after_dead=10 → SEX_ARM_PHASE4_OK.
Negative scan: PHASE4_FAIL, stray_irq, kill_survivor, ap_timeout,
psci_ret, [sex.arm.panic], unexpected=1, repeated entry (reboot loop).

## Static proofs (no runtime claims)
- ELF: machine=183, entry=0x40080000, 0 undefined; symbols verified:
  sex_arm_restore_eret, sex_arm_first, sex_arm_tick, sex_arm_kill_warden,
  sex_arm_ap_entry, sex_arm_ap_main, arm64_phase4_entry, AP_STACK,
  vectors 2048-aligned, __bss_start page-aligned (@0x40087000).
- NOT provable statically: GIC config correctness, IRQ delivery/firing,
  context-switch correctness, AP startup, cache-clean sufficiency,
  tick cadence. ALL deferred to the unchanged closeout mission.

## Gotchas added
15. Timer-before-scheduler-start nondeterminism: enabling CNTP before
    the first task eret risks a pending IRQ pre-empting task A's first
    instruction → B prints first → marker order breaks. Fix: timer::init
    LAST (after SMP) in phase4.rs. Class: any init that unmasks an IRQ
    source must be ordered against first-task entry.
16. ICC sysreg handler trick: x19 (callee-saved, already in the saved
    frame) is free scratch across the tick call for intid — EOIR needs
    it after the switch decision; restore happens after EOI.
17. `let _ = some_generic_fn;` (import-silencing) is a TYPE ERROR
    (E0282) — delete the import instead.
18. PSCI CPU_ON return: x0=0 success; nonzero = PSCI error code —
    print as FAIL marker, never continue silently.

## Next safe phase
ARM64_PHASE_5_PDX_MECHANISM_V1 (plan Phase 5: SVC paths + generic PDX
call mechanism; NO POE claims; real sexsh↔sexfiles round trip requires
the aarch64 userland backend — expected to be recorded as deviation).
