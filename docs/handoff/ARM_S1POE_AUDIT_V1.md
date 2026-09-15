# ARM S1POE AUDIT V1 (2026-07-21) — read before any Arm work

Audit-only mission. No code changed. Full plan:
/Users/xmacbookm5/Projects/Sex_ArmV9/docs/ARM_S1POE_PORT_PLAN_V1.md

Key path-specific findings (verified this tree):
- All PKRU writes are kernel-side only: kernel/src/interrupts.rs
  (syscall_entry ~131-293, timer/page-fault stubs), scheduler.rs:401,
  pku.rs. Userland (sex-pdx, sex-rt, servers, apps) contains ZERO
  wrpkru/rdpkru → on Arm keep CPACR_EL1.E0POE=0 forever; EL0 POR_EL0
  writes trap. Strictly stronger than x86 (WRPKRU is user-executable).
- PDX SyncCall (kernel/src/ipc.rs:117 pdx_call) is a raw `call` in
  ring-0 syscall context with PKRU=GodMode. PDX is EL1-mediated RPC,
  NOT an EL0→EL0 protected call. Preserve this semantic first.
- All 16 PKEYs consumed at boot (pku.rs:1-7 + init.rs:41-105 domain_id
  =i+1 → kaleidoscope=15). Arm key-count policy required (plan §C.5).
- TaskContext is #[repr(C)] + offset assert (scheduler.rs:26), consumed
  by naked asm → arch-internal per-arch layout; do NOT unify.
- ProtectionDomain fields (capability.rs:219-231) read by portable Rust
  → renaming = shared-struct change = STOP FIRST. Keep names; semantic
  aliases live in new kernel/src/protection.rs (Phase 1, dead code).
- crate::pku::PKU_ENABLED is referenced by NAME from naked asm —
  path must keep resolving through any arch shim.
- Memory-order NEEDS-FIX list (Arm weak model): kernel/src/ipc_ring.rs
  SPSC idx/data publication (Relaxed→Release/Acquire); scheduler
  runnable-flag publication; MMIO doorbell fences (DMB ISH via
  fence(Release)). Everything else audited OK (sex-pdx AtomicRing
  Acquire/Release correct).
- Boot contract is Limine (limine.cfg); BOOTAA64.EFI already in repo
  root → keep Limine for aarch64 parity.
- Reference platform (GENERIC BRING-UP ONLY): qemu-system-aarch64
  -M virt,gic-version=3 -cpu max. QEMU S1POE IS UNVERIFIED — official
  QEMU docs list FEAT_S1PIE, not FEAT_S1POE. It becomes an S1POE proof
  platform only after plan §I probes P1–P6 pass. Never write "QEMU
  supports S1POE".
- SyncCall GodMode discovery recorded in
  docs/handoff/PDX_SYNCCALL_GODMODE_SEMANTICS_V1.md — read it before
  touching ipc.rs. Do NOT fix x86 PDX during the Arm port.
