# ARM64 PHASE 6B DECODE CORRECTION + PLATFORM SELECTION V1 (2026-07-22)

Mission: ARM64_PHASE_6B_DECODE_CORRECTION_AND_PLATFORM_SELECTION_V1
(detection/documentation only). Backup: sex_pre_decodefix_20260722.tar.xz
(SHA 3269c4de…). Executed in the permissioned parent session; the nested
run remained sandbox-silent (third instance of the pattern).

## A. PASS

## B. CORRECTED FIELD TABLE (ID_AA64MMFR3_EL1)
Encoding-truth source: Arm-maintained architectural constants — Arm
Trusted Firmware include/arch/aarch64/arch.h
(ID_AA64MMFR3_EL1_S1POE_SHIFT = 16), matching the user's authorized
table. Implementation-truth cross-check: QEMU v11.0.2
target/arm/cpu-features.h:333-347 (NOT described as the ARM ARM).
  [3:0]  TCRX    (new constant, shift 0)
  [7:4]  SCTLRX  (new constant, shift 4)
  [11:8] S1PIE   (kept, shift 8)
  [15:12] S2PIE  (new constant, shift 12)
  [19:16] S1POE  (CORRECTED 4 -> 16)
The misleading ID_AA64MMFR3_TCR2_SHIFT label is REMOVED (the TCR2
feature field is TCRX at [3:0]). Raw register printing preserved;
behavior wording stays UNPROVEN; fail-closed policy, zero POR/TCR2/CPACR
POE writes, provisional POIndex policy and Phase-7-deferred behavior
all preserved.

## C. FILES CHANGED
- kernel/src/arch/aarch64/registers.rs (constants + provenance comment)
- kernel/src/arch/aarch64/poe.rs (labels + VERIFIED-positions note)
- scripts/arm64_phase5_gate.sh (decode regression test added)
- docs/handoff/ARM64_S1POE_PLATFORM_SELECTION_V1.md (new)
- docs/handoff/ARM64_PHASE_6B_DECODE_CORRECTION_...V1.md (this file)
- mirrors to Sex_ArmV9/docs/handoff/
No production source touched; no POR/TCR2 implementation; no POIndex
policy change.

## D. REGRESSION TEST (in arm64_phase5_gate.sh, runs every gate)
Python extracts the five shift constants from registers.rs and proves
decode(0x1000000011001111) == {tcrx:1, sctlrx:1, s1pie:1, s2pie:1,
s1poe:0}; FAILS if S1POE shift == 4; FAILS if the TCR2 label returns.
Verified live: "[gate56] decode regression ok: {'tcrx': 1, 'sctlrx': 1,
's1pie': 1, 's2pie': 1, 's1poe': 0}".

## E. ARM RUNTIME RESULT (gate rerun, QEMU 11.0.2 canonical cmd)
- Full Phase 2->6 baseline: PASS (runtime=PASS), no faults/reboot.
- Raw ID printed: id_aa64mmfr3=0x1000000011001111.
- Corrected line observed at runtime:
  [sex.arm.poe] fields=VERIFIED(arm_tfa+qemu_xcheck) behavior=UNPROVEN
  tcrx=1 sctlrx=1 s1pie=1 s2pie=1 s1poe=0
- S1POE reports ABSENT; isolation=none dev=1; Phase 7 remains blocked;
  no POR/TCR2 probe attempted (zero such writes in the build).
- Provenance: the revoked "S1POE REPORTED" claim and the reason
  (provisional positions) are preserved in the closeout doc's §E.

## F. PLATFORM STRATEGY
See docs/handoff/ARM64_S1POE_PLATFORM_SELECTION_V1.md:
QEMU 11.0.2 = generic-Arm platform (PASS) but S1POE-absent;
FVP/AEM = preferred next target (model/release NOT selected, no claim
until recon); real hardware = later, ID+P2-P6 evidence required;
S1PIE = not a replacement (user decision). FVP reconnaissance package
included (image path, recon markers, model/param capture, STOP FIRST
rules for platform-constant drift, no arbitrary-FVP claims).

## G. GATES
- arm64_phase5_gate.sh: PASS (runtime=PASS) incl. new decode regression
- x86 build inside gate: GREEN (pinned 153 warnings, same toolchain)
- x86 pinned files: byte-identical (gate-verified)
- no ABI/PDX/scheduler/TaskContext/ProtectionDomain changes
- no POR/TCR2/CPACR POE writes anywhere (gate-enforced)
- no production POIndex assignment
- x86 runtime lane NOT rerun (no shared file changed; kernel/Cargo.toml
  untouched this mission)

## H. NEXT STATUS
Phases 2-5: runtime CLOSED (QEMU). 6A: runtime PASS with corrected
labels. 6B: BLOCKED_BY_PLATFORM (S1POE absent on QEMU). 7: FORBIDDEN
until an FVP/model reports S1POE != 0 AND P2-P6 pass. Next investment:
obtain/configure a proven-S1POE FVP and run the reconnaissance package
— not more Sex kernel protection code.

FINAL RETURN: ARM64_S1POE_FVP_RECONNAISSANCE_REQUIRED
