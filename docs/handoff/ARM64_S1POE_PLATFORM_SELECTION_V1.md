# ARM64 S1POE PLATFORM SELECTION V1 (2026-07-22)

Status of S1POE execution platforms after the decode correction
(ARM64_PHASE_6B_DECODE_CORRECTION_AND_PLATFORM_SELECTION_V1). No platform
is accepted without raw ID_AA64MMFR3_EL1 evidence AND P2-P6 behavior.

## 1. QEMU 11.0.2 (this host, Homebrew, TCG)
- Generic Arm runtime platform (Phases 2-5 + 6A detection): **PASS** —
  canonical: -M virt,gic-version=3,virtualization=on -cpu max -smp 2.
- S1POE field (corrected decode [19:16] of 0x1000000011001111): **ABSENT**.
- POR_EL0/POR_EL1/TCR2_EL1 sysregs: **NOT IMPLEMENTED** (no cpregs.h
  definitions; only EL2 trap-control FIELD names mention them).
- Translation walk (target/arm/ptw.c): zero POE/POR handling.
- QEMU master (fetched 2026-07-22): still no S1POE implementation.
- QEMU's own docs warn advertised CPU features may be partially
  supported — this mission is the living proof (ID bits ahead of
  implementation).
- Verdict: P2-P6 UNSUITABLE here. Remains the generic-Arm platform.

## 2. Arm FVP / AEM (Architecture Envelope Model) — PREFERRED NEXT TARGET
- Arm positions FVPs/AEM as programmer-visible simulations that can
  expose architecture features before silicon exists.
- Exact model + release: **NOT YET SELECTED**. Feature availability
  varies by model/release — no support claim until the raw ID register
  and register behavior are tested on the chosen model.
- FVPs run on x86_64 Linux/Windows hosts; this aarch64 Mac needs a
  remote/Linux host or VM for FVP execution.
- Acceptance gate: S1POE != 0 at [19:16] AND P2-P6 pass.

## 3. Real hardware
- No board claimed. Arm's public Linux feature tracker still describes
  S1POE support as ongoing — platform immaturity expected.
- Requires on-device ID_AA64MMFR3_EL1 read plus full P2-P6 evidence.

## 4. S1PIE — NOT A REPLACEMENT (user decision, 2026-07-22)
- Do not pivot Sex isolation to S1PIE: it is indirect base-permission
  encoding, NOT a per-context overlay-register mechanism analogous to
  PKRU. It cannot provide POR-style hot domain switching by itself.
- May remain useful later for page-table base-permission indirection.
- Note: QEMU 11.0.2 reports S1PIE=1 but also lacks PIRE0/PIR sysregs.

## FVP RECONNAISSANCE PACKAGE (docs/scripts only; kernel UNMODIFIED)
Run FIRST on any candidate FVP, before any probe adaptation:

1. Image: the UNCHANGED chained image
   target/aarch64-sex/release/sex-kernel-arm64 (rebuild via
   scripts/build_arm64_phase2.sh on the FVP host if needed).
2. Model/version capture (record in the recon log):
   - model binary name + --version output
   - full parameter list used (the model's --list-params or equivalent
     dump) — saved verbatim.
3. Boot with the model's closest equivalent of the QEMU virt layout;
   capture serial to a log.
4. Required serial output (recon markers):
   - [sex.arm.entry] current_el=...
   - [sex.arm.dtb] ...
   - SEX_ARM_ALIVE
   - [sex.arm.poe] id_aa64mmfr3=0x...   <-- THE recon target
   - the corrected fields line (s1poe= must be nonzero to proceed)
5. STOP FIRST (do NOT boot further, do NOT adapt probes) if any of:
   - UART base/IRQ, RAM base/size, GIC base, or kernel load address
     differ from QEMU virt (0x09000000 PL011, 0x40000000 DRAM,
     0x08000000 GICD / 0x080A0000 GICR, 0x40080000 load): platform
     constants must be adapted as a SEPARATE reviewed change first;
   - the model does not reach SEX_ARM_ALIVE;
   - id_aa64mmfr3 reads 0 or s1poe=0.
6. NO CLAIM that an arbitrary Armv9 FVP implements S1POE — the recon
   exists precisely to prove or disprove it per model/release.
7. Only after s1poe != 0 on the model: adapt the bounded P2-P6 probe
   harness (ARM64_PHASE_6B_S1POE_RUNTIME_PROBES_V1 prompt, saved
   verbatim) to the model's platform constants and run it.
8. Phase 7 begins only after P2-P6 PASS on that model.

## Honest branches if no accessible FVP implements S1POE
A. Implement S1POE in QEMU as a separate upstream/emulator project.
B. Obtain an Arm Fast Models configuration that includes S1POE.
C. Freeze Phase 7 until suitable silicon appears.

The generic AArch64 conversion (Phases 2-5 runtime CLOSED on QEMU) is
unaffected and remains the proven base. Only the MPK-equivalent
hardware-isolation layer waits for an execution platform.
