# PDX SYNCCALL GODMODE SEMANTICS V1 (2026-07-21)
Permanent record of the Arm-port audit's most important x86 discovery.
DO NOT FIX THIS DURING THE ARM PORT. Documentation only.

## Facts (verified in this tree)
- `GraphEdge::SyncCall` (kernel/src/ipc.rs:135) resolves to
  `unsafe fn pdx_call(entry_point, …)` (kernel/src/ipc.rs:117): a raw
  `call` to the callee entry point executed FROM ring-0 syscall-handler
  context.
- `syscall_entry` (kernel/src/interrupts.rs, quickmap lines 131–293)
  establishes PKRU = 0x00000000 (God Mode) immediately after saving the
  user PKRU. The SyncCall callee therefore executes with UNRESTRICTED
  memory access, at CPL0, on the caller's kernel syscall stack.
- Capability validation DOES occur (`resolve_edge`, ipc.rs:144+) before
  the call — the edge is capability-gated.
- But callee MEMORY EXECUTION is not isolated by caller/callee PKRU
  masks: there is no per-callee PKRU transition on the SyncCall path.
  PKRU isolation applies only to PDs running their own scheduled
  threads (scheduler.rs:319–401 bind→wrpkru→switch, syscall entry/exit
  save/restore).

## What the current x86 model actually claims
- Fault-separated scheduled protection domains: YES (PKRU per-thread,
  warden on violation).
- Hostile native-code containment DURING a SyncCall: NO.
- Additionally, on real x86 hardware WRPKRU is user-executable, so any
  ring-3 PD can self-grant God Mode at any time. Userland currently
  never does this (zero wrpkru/rdpkru in sex-pdx/sex-rt/servers/apps),
  but nothing prevents it. The x86 model is trusted-but-faulty PDs.

## Arm port consequences
- Arm Phase 5 preserves this semantic exactly (EL1-mediated RPC), under
  the `arm64-no-poe-development` config, with NO isolation claims.
- Arm Phase 7 must NOT be forced to preserve this weakness. The Phase-7
  design (plan §E.1) runs the callee at EL0 under a caller/callee pair
  overlay with execute gating — the first genuinely hardware-isolated
  PDX fast path in Sex. This is a deliberate, documented IMPROVEMENT
  over x86, not a semantic mismatch.
- Arm boots with CPACR_EL1.E0POE=0 forever: EL0 POR_EL0 writes trap to
  EL1, closing the "user WRPKRU" hole that x86 cannot close.

## Forbidden during the Arm port
- Do not modify SyncCall, resolve_edge, traverse_edge, or pdx_call.
- Do not redesign PDX.
- Do not describe the current x86 path as hardware-isolated in any doc,
  gate output, or commit message.
- Any future x86 execute-gated PDX redesign is a SEPARATE STOP-FIRST
  mission with its own audit.
