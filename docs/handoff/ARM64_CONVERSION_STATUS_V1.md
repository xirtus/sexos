# ARM64 CONVERSION STATUS V1 (2026-07-22) — read FIRST before any Arm work

## *** CURRENT STATE (2026-07-22, runtime-verified; P1 CORRECTED) ***
LANE A: ARM_PHASE_2_6_RUNTIME_PASS — the full chained image boots
Phases 2->6 on QEMU 11.0.2 (see ARM64_PHASE_2_6_RUNTIME_CLOSEOUT...
and ARM64_RUNTIME_FIXES_V1.md for the 6 runtime-found kernel fixes).
LANE B: X86 PKU runtime REPRODUCED on this host (CR4.PKE, keyed PDs,
scheduler, clock; ORDER_GATE RED = pre-existing x86 debt, untouched).
*** P1 CORRECTION (Phase 6B architecture verification, 2026-07-22) ***:
the provisional ID_AA64MMFR3 decode was WRONG — S1POE is [19:16] not
[7:4] (that nibble is SCTLRX). Corrected decode of 0x1000000011001111:
S1POE=0 S2POE=0 (ABSENT); S1PIE=1 S2PIE=1 TCRX=1 SCTLRX=1 present-but-
unimplemented in QEMU sysregs. The "S1POE REPORTED" claim is REVOKED.
QEMU 11.0.2 AND master implement NO S1POE (no POR/TCR2_EL1 sysregs,
no POE in the walk). Phase 6B/7 BLOCKED on platform. Verdict of the
6B mission: STOP_FIRST_ARCHITECTURE_MISMATCH — see
ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1.md for the verified layout,
evidence, and the two decisions required (decode-constant correction
authorization; platform strategy: FVP / future QEMU / silicon / S1PIE).
NEXT: user decision on those two items. Phase 7 production POR_EL0
switching stays BLOCKED. E0POE=0 forever. POIndex=0.

Master status for the Sex → AArch64 (FEAT_S1POE) port. Goal: "finish
conversion using the planned phases" — executed to the maximum extent
possible on a host with NO QEMU binaries. Every phase is code-complete
that CAN be code-complete without runtime facts; everything else is
honestly deferred, not silently dropped.

## Phase board
| Phase | Name | Static+Build | Runtime | Notes |
|---|---|---|---|---|
| 0/1 | freeze + arch seam | PASS | n/a | x86 baseline frozen |
| 2 | entry/serial | PASS | **PASS (QEMU)** | EL2 drop proven; DTB absent-by-design (ELF boot) |
| 3 | MMU/global VAS | PASS | **PASS (QEMU)** | W^X + 3 EL0 negatives contained at runtime |
| 4 | GIC/timer/sched | PASS | **PASS (QEMU)** | ping-pong + kill containment + AP boot proven |
| 5A | EL0/SVC mechanism | PASS | **PASS (QEMU)** | echo + ERR_CAP_INVALID exact at runtime |
| 5B | full Sex PDX semantic parity | NOT YET PROVEN | — | capability.rs/ipc.rs untouched by design; no PDX-parity claim |
| 6A | S1POE detect/tag scaffolding | PASS | **PASS (QEMU)** | P1 done: ID=0x1000000011001111 |
| 6B | S1POE runtime probes P1-P6 | **UNBLOCKED** | — | mission to author; probe-code scope change needed (gate static) |
| 7 | POR_EL0 switch | BLOCKED | — | until P2-P6 pass |
| 8 | isolation/perf proof | BLOCKED (runtime-only) | — | needs 7 |
| 9 | hardware closeout | BLOCKED (hardware) | — | needs S1POE silicon/FVP |

## Canonical gates (run in order)
- scripts/arm64_phase2_gate.sh — base quick gate (current-tree safe)
- scripts/arm64_phase5_gate.sh — FULL current-tree gate (superset:
  phases 2-6 statics, x86 153-warning pin, aarch64 build, ELF/symbols,
  payload assembler cross-checks, runtime scan when QEMU exists)
- scripts/arm64_phase3_gate.sh / arm64_phase4_gate.sh — HISTORICAL
  (scope-pinned to their phase backups; later phases changed pinned files)
Current state of both canonical gates: PASS (runtime=SKIP_NO_QEMU).

## Runtime state (2026-07-22 — host provisioned, both lanes EXECUTED)
1. Closeout EXECUTED on this host: Lane A PASS (full 2->6 chain),
   Lane B PASS-with-preexisting-ORDER_GATE-RED, P1 = S1POE REPORTED.
   Canonical gate: bash scripts/arm64_phase5_gate.sh -> PASS (runtime=PASS).
2. NEXT: ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1 — bounded P2-P6 probe
   code (POR/TCR2 writes inside a DESIGNATED probe module; amend the
   gate's no-write static for that module only). Then Phase 7 may start
   ONLY after all probes pass.
3. X86_PKU_RUNTIME_BASELINE: PKU runtime reproduced on this host
   (2026-07-22); ORDER_GATE RED remains pre-existing x86 debt.
S1POE = REPORTED-but-UNPROVEN (P2-P6 owed). POIndex assignments = PROVISIONAL.
Host toolchain additions: QEMU 11.0.2 (both archs), ripgrep 14.1.1,
xorriso 1.5.8.pl02, limine v7.x-binary native.

## Boot chain (single image, one boot runs all phases)
entry (EL2→EL1) → [phase2] alive/dtb/uart → [phase3] MMU on, W^X, HHDM,
EL0 smoke negatives → [phase4] GIC+timer+2-task ping-pong+kill+PSCI AP
→ [phase5] timer off, EL0 SVC echo/cap_check ×3, ERR_CAP_INVALID proof
→ [phase6] ID_AA64MMFR3 parse, POE_TAGGED(lane0), PHASE6_OK →
[p7 deferred marker] → WFE.
Terminal markers on success: SEX_ARM_PHASE6_OK + [sex.arm.p7] deferred=1.

## Hard rules still in force
- No POR/TCR2/CPACR-E0POE writes (gate-enforced). CPACR_EL1 stays 0.
- capability.rs/ipc.rs/sex-pdx/pku.rs/scheduler.rs etc. byte-pinned vs
  pre-phase2 backup. lib.rs/arch.rs/Cargo.toml changed ONLY per phase
  scopes (feature additions, cfg-gating) — scope pins in gates.
- No per-PD TTBR. Single global VAS. POIndex stays 0 until probes pass.
- x86 build: GREEN, exactly 153 warnings under pinned toolchain
  (rustc 1.99.0-nightly 0e29c21d9 / cargo 3efb1f477).
- Hand-encoded payloads are assembler-verified by the gates (clang).

## Backups (newest first)
- sex_pre_runtimefix_20260722.tar.xz (pre-runtime-fix tree)
- sex_pre_phase5_20260722_004205.tar.xz (SHA c392dcf7…)
- sex_pre_phase4_20260722_001220.tar.xz (SHA 08172d6f…)
- sex_pre_phase3_20260721_234142.tar.xz (SHA d4ab0b10…)
- sex_pre_phase2_20260721_220749.tar.xz (SHA 0fb670bf…)

## Handoffs (docs/handoff/)
ARM64_PHASE_2_ENTRY_SERIAL_V1.md, ARM64_PHASE_3_MMU_GLOBAL_VAS_V1.md,
ARM64_PHASE_4_GIC_TIMER_SCHED_V1.md, ARM64_PHASE_5_PDX_MECHANISM_V1.md,
ARM64_PHASE_6_DETECT_TAG_V1.md,
ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1.md,
ARM_S1POE_AUDIT_V1.md, PDX_SYNCCALL_GODMODE_SEMANTICS_V1.md —
mirrored to Sex_ArmV9/docs/handoff/.
