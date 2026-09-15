# ARM64_PHASE_5_PDX_MECHANISM_V1 — Handoff (2026-07-22)
(plan alias: ARM_PHASE5_PDX_GENERIC_V1)

## Result
STATIC+BUILD: **PASS**. RUNTIME: **SKIP_NO_QEMU** (never converted).
x86 unchanged and GREEN. X86_PKU_RUNTIME_BASELINE_PENDING=1 owed.

## Backup
- Sex_ArmV9/backups/sex_pre_phase5_20260722_004205.tar.xz
- SHA-256 (first 16): c392dcf79d321e3a3b1048bb91d0aa23…

## What this phase is (and is NOT)
Plan Phase 5 = "Generic PDX on AArch64 (NO POE claims)".
DEVIATION (recorded, deliberate): the plan gate (sexsh→sexfiles PDX
round trip) is IMPOSSIBLE on this tree — no aarch64 userland backend
exists (plan table B: userland parity is a later USERLAND phase), and
porting capability.rs/ipc.rs to aarch64 requires untangling x86-only
deps (pku/scheduler/interrupts) that the Phase-1 audit forbids touching
during these phases. This phase therefore implements the MECHANISM PDX
needs, with the plan's negative gate preserved EXACTLY:
  1. EL0 → SVC → EL1 → EL0 round trip, args x0-x1 in, result x0 out,
     full 272-byte frame save/restore (same frame as the IRQ path).
  2. NEGATIVE GATE: bad cap id → ERR_CAP_INVALID = 0xFFFF_FFFF_FFFF_FFFC
     — the exact sex-pdx ABI constant, mirrored in registers.rs and
     GATE-CHECKED against crates/sex-pdx/src/lib.rs on every run.
  3. good cap id → 0.
NO PDX-parity claim is made. capability.rs/ipc.rs are UNTOUCHED
(gate-verified vs pre-phase2 backup). protection.rs stays dead.
sex-pdx ABI untouched (no aarch64 build of it added).

## Design
- exceptions.rs slot 0x400 now pre-dispatches on ESR.EC: 0x15 (SVC64) →
  full-frame SVC path (sex_arm_svc in phase5.rs); everything else →
  Phase-3 smoke warden (preserved). ESR is read via x9 with save/restore
  (payloads keep x9 live).
- SVC return convention: result written into the frame's x0 slot, the
  FRAME's ELR slot advanced +4 (skip svc), shared restore eret.
- Slot 0x480 (IRQ from lower EL) now routes to the real IRQ handler —
  a late timer IRQ during EL0 phases is handled correctly (frame
  save/restore resumes the EL0 context).
- Phase-5 EL0 payload2 (phase5.rs, 18 hand-encoded words): echo(41) →
  mailbox[2]; cap_check(0xDEAD) → mailbox[3]; cap_check(42) → mailbox[4];
  tag=5; deliberate fault at 0x60000000. Warden (tag 5) verifies
  EC=0x24 + FAR + all three mailbox results, prints markers, chains via
  ERET to arm64_phase6_entry.
- Chain: task A (after SEX_ARM_PHASE4_OK) → phase5::run() →
  timer::disable() (no further scheduling; SVC test uninterruptible) →
  enter2 → payload2 → warden → PHASE5_OK → chain to Phase 6.
- Timer disable means Phase-4's ping-pong is over by design; a
  resurrected-task hazard was avoided by entering EL0 from task A's
  context (scheduler still sees CUR=A; a stray IRQ would resume the
  SVC-test frame correctly).

## Files changed
NEW: arch/aarch64/phase5.rs, scripts/arm64_phase5_gate.sh, handoffs.
EDIT: exceptions.rs (SVC dispatch + tag5 + p6 chain + slot 0x480),
sched.rs (task A → phase5::run), timer.rs (disable), mmu.rs (map p5
page), early.rs (prepare p5 payload), el0_smoke.rs (mailbox_slot,
enter_at), registers.rs (phase-5/6 consts), mod.rs (+3), Cargo.toml
(feature only: arm64-require-poe).
Scope-pinned unchanged vs pre-phase5 backup (gate): lib.rs, src/main.rs,
arch.rs, serial.rs, aarch64-linker.ld, aarch64-sex.json.

## Expected runtime markers (ordered; gate enforces)
… → SEX_ARM_PHASE4_OK → [sex.arm.p5] entry=1 → [sex.arm.timer] off=1 →
[sex.arm.el0] enter2=1 → [sex.arm.el0] done2=1 → [sex.arm.svc] echo=ok →
[sex.arm.svc] cap_bad=ERR_CAP_INVALID → [sex.arm.svc] cap_good=ok →
SEX_ARM_PHASE5_OK → (phase 6 markers, see its handoff).
Negative: PHASE5_FAIL, unexpected=1, stray_irq, [sex.arm.panic], etc.

## Static proofs (no runtime claims)
- Payload2: 18/18 words == clang assembler output (the gate CAUGHT and
  I fixed a real bug: STR [x9,#imm] imm12 scaling — see gotcha 19).
- ELF: all Phase-2..5 symbols present (sex_arm_svc, arm64_phase6_entry,
  …), 0 undefined, vectors 2048-aligned.
- NOT provable statically: SVC delivery, frame correctness on the SVC
  path, mailbox verification, ERET chaining. Deferred to closeout.

## Gotchas added
19. STR/LDR unsigned-offset imm12 is SCALED by access size (offset/8 for
    64-bit): str x0,[x9,#16] = 0xF9000920, NOT 0xF9080120. The
    assembler cross-check caught this — never hand-wave encodings the
    gate can verify.
20. Nested heredocs: a heredoc body containing a lone `PY` line
    terminates the outer `<<'PY'` early — zsh then parses Rust as shell.
    Write long scripts directly instead of generating them.

## Next safe phase
ARM64_PHASE_6_DETECT_TAG_V1 — implemented alongside (same gate run);
see ARM64_PHASE_6_DETECT_TAG_V1.md. Phase 7 is DEFERRED BY DESIGN
(runtime probes P1-P6 required first).
