# ARM64_PHASE_6_DETECT_TAG_V1 — Handoff (2026-07-22)
(plan alias: ARM_PHASE6_DETECT_TAG_V1)

## Result
STATIC+BUILD: **PASS**. RUNTIME: **SKIP_NO_QEMU** (never converted).
S1POE status: **UNVERIFIED** (unchanged — no claim made, none possible).
x86 unchanged and GREEN.

## Backup
- Sex_ArmV9/backups/sex_pre_phase5_20260722_004205.tar.xz (covers 5+6)

## Scope honesty (read first)
Plan Phase 6 = "S1PIE/S1POE detection + page tagging". What exists:
- poe.rs: reads ID_AA64MMFR3_EL1 (always safe), prints raw value +
  parsed S1POE/S1PIE/TCR2 fields. **FIELD POSITIONS ARE UNVERIFIED** —
  commonly documented positions (S1POE=[7:4], S1PIE=[11:8], TCR2=[15:12])
  are used and MUST be confirmed against the ARM ARM / runtime probes
  P1-P6 before any reliance. Zero-field = feature-absent (fail-safe).
- Fail-closed path: cfg feature `arm64-require-poe` → if S1POE field
  reads 0, prints SEX_ARM_FAILCLOSED_NO_S1POE and halts. Default build
  (feature off) prints isolation=none dev=1 and continues.
- "Tagging": mmu.rs descriptor plumbing can carry a POIndex field, but
  EVERY descriptor uses POIndex 0 (default lane) — safe on all hardware,
  including hardware where POIndex bits are RES0. SEX_ARM_POE_TAGGED is
  printed with poindex_default=0 nonzero_tags=0. **No isolation claim.**
- NO POR_EL0/POR_EL1/TCR2_EL1 writes anywhere (gate greps for `msr`
  forms; detection READS are allowed).
- CPACR_EL1 untouched (still 0 from entry; E0POE-adjacent safety gate
  verifies no CPACR writes with any bit set).

## Phase 7 status: DEFERRED BY DESIGN
Plan Phase 7 (POR_EL0 switching) requires Phase-6 RUNTIME facts:
POIndex width, POR lane encoding, PIE/POE interaction, POR-only-no-TLBI
proof — probes P1-P6 on a QEMU-capable host. The plan's own STOP FIRST
rule forbids writing it blind. The boot chain therefore ends Phase 6
with: [sex.arm.p7] deferred=1 reason=runtime_probes_required → WFE.
Phase 7 implementation starts only after the closeout mission proves
the facts. POIndex→domain assignments remain PROVISIONAL (plan §C.5).

## Expected runtime markers (ordered; gate enforces)
… SEX_ARM_PHASE5_OK → [sex.arm.p6] entry=1 →
[sex.arm.poe] id_aa64mmfr3=0x… → [sex.arm.poe] fields=UNVERIFIED s1poe=N
s1pie=N tcr2=N → (s1poe=0: [sex.arm.poe] isolation=none dev=1) →
SEX_ARM_POE_TAGGED poindex_default=0 nonzero_tags=0 →
SEX_ARM_PHASE6_OK → [sex.arm.p7] deferred=1 …
Negative: FAILCLOSED (in default builds), PHASE5_FAIL, FATAL, etc.

## Files changed (beyond Phase 5's set)
NEW: arch/aarch64/poe.rs, arch/aarch64/phase6.rs.
EDIT: mod.rs (+poe, phase6), Cargo.toml (arm64-require-poe feature only).
Detection constants live in registers.rs (marked UNVERIFIED).

## Static proofs (no runtime claims)
- Build green; ELF symbols arm64_phase6_entry present; zero POR/TCR2
  writes (gate grep); ERR_CAP_INVALID matches sex-pdx; x86 untouched.
- NOT provable: ID field positions, feature presence, any POE behavior.

## Next safe phase
ARM64_PHASE_7_POR_SWITCH_V1 — BLOCKED on runtime probes P1-P6 (closeout
mission). Phases 8/9 are runtime/hardware proof — nothing implementable
on a QEMU-less host.
