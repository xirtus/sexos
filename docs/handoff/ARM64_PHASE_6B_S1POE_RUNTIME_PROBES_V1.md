# ARM64 PHASE 6B S1POE RUNTIME PROBES V1 (2026-07-22) — STOP FIRST

Mission: ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1 (prompt saved verbatim,
Sex_ArmV9/prompts/). Backup before work: sex_pre_phase6b_20260722.tar.xz
(SHA da0e21ee…). NO probe code was written — the mission's own
ARCHITECTURE VERIFICATION step fired its STOP FIRST condition BEFORE any
implementation: the provisional field positions are contradicted by the
architecture layout, and with the CORRECTED positions the platform does
not have S1POE at all.

## A. VERDICT: STOP_FIRST_ARCHITECTURE_MISMATCH (Phase 7 BLOCKED)

## B. ARCHITECTURE CONSTANTS — VERIFIED LAYOUT (and the correction)
Source: QEMU v11.0.2 target/arm/cpu-features.h lines 333-347 (QEMU's
ID_AA64MMFR3 field table mirrors the ARM ARM; cross-checked against
QEMU master fetched 2026-07-22 — identical layout). No Linux source used.

ID_AA64MMFR3_EL1 correct layout:
  [3:0]=TCRX  [7:4]=SCTLRX  [11:8]=S1PIE  [15:12]=S2PIE
  [19:16]=S1POE  [23:20]=S2POE  [27:24]=AIE  [31:28]=MEC
  [35:32]=D128  [39:36]=D128_2  [43:40]=SNERR  [47:44]=ANERR
  [51:48]=RES0  [55:52]=SDERR  [59:56]=ADERR  [63:60]=SPEC_FPACC

PRIOR PROVISIONAL DECODE (registers.rs, marked UNVERIFIED): S1POE=4,
S1PIE=8, TCR2=12 — WRONG. [7:4] is SCTLRX, [15:12] is S2PIE, and the
TCR2 feature field is TCRX at [3:0]. The Phase-6 print
"s1poe=1 s1pie=1 tcr2=1" actually read SCTLRX=1, S1PIE=1 (correct),
S2PIE=1. THE "S1POE REPORTED" CLAIM OF THE 2026-07-22 CLOSEOUT IS
HEREBY REVOKED — it was based on the wrong nibble, which is exactly
why the positions carried the UNVERIFIED marking.

CORRECT DECODE of the observed value 0x1000000011001111
(QEMU 11.0.2 -cpu max):
  TCRX=1  SCTLRX=1  S1PIE=1  S2PIE=1
  S1POE=0  S2POE=0          <-- S1POE ABSENT
  AIE=1   MEC=1    D128=0   SPEC_FPACC=1

## IMPLEMENTATION DEPTH (QEMU v11.0.2 AND master, 2026-07-22)
- POR_EL0 / POR_EL1 sysregs: NOT DEFINED in cpregs.h (only HFGRTR/
  HFGWTR trap-control FIELD names reference them). Any MRS/MSR to
  POR_EL0/POR_EL1 traps UNDEFINED. -> P2 impossible.
- TCR2_EL1 sysreg: NOT DEFINED (despite TCRX=1 in the ID; QEMU max
  advertises ID bits ahead of implementation). -> P3 impossible.
- Translation walk (target/arm/ptw.c): ZERO POE/POR handling on
  v11.0.2 and on master. -> P4/P5 impossible.
- CPACR_EL1.E0POE enforcement: nothing to enforce. -> P6 impossible.
- S1PIE: reported =1 but PIRE0_EL1/PIR_EL1 sysregs likewise NOT
  DEFINED in v11.0.2 (advertise-ahead behavior again).
- QEMU master (fetched 2026-07-22): still no S1POE implementation.

## D-H. P2/P3/P4/P5/P6 RESULTS
NOT RUN — impossible on this platform (registers unimplemented;
writes would trap UNDEFINED, reads cannot round-trip). No probe code
exists; poe_probe.rs was NOT created; the probe feature was NOT added;
the gate's no-POR/TCR2-write static was NOT amended (nothing to exempt).

## I. RESTORE / FAULT SCAN
Nothing executed; production image unchanged. Both canonical gates
re-verified PASS (runtime=PASS) on the untouched tree.

## J. GATES
- arm64_phase2_gate.sh: PASS (runtime=PASS)
- arm64_phase5_gate.sh: PASS (runtime=PASS)
- x86 build: GREEN (pinned 153 warnings) — no shared file changed.

## K. NEXT STATUS / DECISIONS REQUIRED (STOP FIRST items for the user)
1. AUTHORIZE decode-constant correction (source change outside the
   mission's allowed files — refused without authorization):
   registers.rs ID_AA64MMFR3_S1POE_SHIFT 4->16, add SCTLRX=4,
   S1PIE stays 8, S2PIE=12, TCRX=0 (rename from TCR2), and the
   phase6/poe.rs print labels accordingly. Zero runtime behavior
   change (detection-only path; zero field = absent fail-safe).
2. PLATFORM STRATEGY for S1POE (Phase 6B/7 cannot proceed without one):
   a. Arm FVP (Fixed Virtual Platform) modeling Armv9.4 FEAT_S1POE —
      FVPs run on x86_64 Linux/Windows hosts (this Mac would need a
      remote/Linux host or VM);
   b. future QEMU with S1POE (absent in 11.0.2 and master today);
   c. real Armv9.4 silicon with FEAT_S1POE;
   d. re-evaluate S1PIE as the isolation mechanism (also not
      implemented in QEMU 11.0.2 sysregs, despite the ID bit).
3. Phase 7 remains BLOCKED. E0POE=0 forever. POIndex=0 everywhere.
   Phases 2-5 runtime CLOSED; 6A detection runtime PASS (label
   correction owed); 6B blocked on platform.

FINAL RETURN: STOP_FIRST_ARCHITECTURE_MISMATCH
