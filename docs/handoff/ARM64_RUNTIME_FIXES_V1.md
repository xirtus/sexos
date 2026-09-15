# ARM64 RUNTIME FIXES V1 (2026-07-22) — bugs found BY RUNTIME, fixed, proven

First-ever runtime of the chained Phase 2-6 image (QEMU 11.0.2, TCG,
-M virt,gic-version=3,virtualization=on -cpu max -smp 2 -m 512M).
Six kernel bugs + three gate bugs surfaced; static review had caught
none of them. Every fix is marked "runtime-found 2026-07-22" in code.
Backup before fixes: Sex_ArmV9/backups/sex_pre_runtimefix_20260722.tar.xz

## Kernel bugs (all in kernel/src/arch/aarch64/)
1. exceptions.rs — warden tag-3 (exec-data negative): ELR+4 skip landed
   at 0x410004, STILL on the NX data page -> double instruction abort ->
   MISMATCH -> PHASE3_FAIL. Fix: new warden return code 3 -> asm
   redirects ELR to the code-page continuation 0x400030
   (USER_CODE_VA+0x30, payload word 12). Rule: instruction-abort skips
   can NEVER use ELR+4 onto a non-executable page.
2. early.rs — DTB: QEMU starts an ELF -kernel at e_entry with x0=0 AND
   LOADS NO FDT INTO GUEST RAM (proven: both EL1-direct and EL2 boots
   show x0=0; full 512 MiB pmemsave scan contains no FDT magic). The
   x0=FDT contract is for raw Image boots. Fix: honest marker
   "[sex.arm.dtb] x0=0 present=0 note=elf_boot_no_fdt"; validate only
   when x0!=0. Platform addresses stay fixed constants.
3. smp.rs — PSCI conduit: QEMU TCG implements PSCI via SMC intercepted
   in its CPU loop; HVC from EL1 traps to the GUEST's EL2 (VBAR_EL2=0)
   -> BSP spun forever (no psci_ret, no timeout). Fix: smc #0.
   (KVM/real hardware uses HVC — platform here is QEMU TCG.)
4. (config) -smp 2 was missing: PSCI CPU_ON for core 1 returned
   -2 INVALID_PARAMETERS. Fixed in the gate's canonical command.
5. gicv3.rs — GICR_IGROUPR0 never written: PPI 30 stayed Group 0 while
   only ICC_IGRPEN1=1 -> Group-0 PPI is NEVER signaled -> no tick ever
   fired (task A monopolized; a_run=1..3 with zero switches). Fix:
   IGROUPR0 |= 1<<30 before ISENABLER0.
6. exceptions.rs — SVC return path advanced frame ELR by +4, but for
   SVC (unlike aborts) ELR_EL1 ALREADY points after the svc (this is
   why Linux el0_svc never adjusts it). The +4 skipped the result-store
   word: mailbox slots read 0 despite correct SVC results. Fix: no ELR
   advance on the SVC path. (Aborts still skip +4 — correct and
   runtime-proven by the Phase-3 negatives.)

## Gate bugs (scripts/arm64_phase5_gate.sh)
7. Marker order: '[sex.arm.timer] freq=' was listed before the SMP
   markers, but phase4_entry runs timer::init LAST by design. Fixed to
   runtime order: ap_boot=1, SEX_ARM_AP_OK, ap_ack=1, freq=.
8. Marker order: strict a1,b1,a2,b2 interleave is impossible — each
   task burns its 3 prints inside one 10ms tick slice (CPU-speed prints
   vs 100Hz switches). Canonical runtime order: a1..a3, b1..b3. The
   proof is the switch + kill containment + survivor completion.
9. RUNTIME=SKIP_NO_QEMU printed even on runtime FAIL. Now RUNTIME=FAIL.
10. arm64_phase2_gate.sh QEMU line predated the chain (no
    virtualization=on, no -smp 2) and its negative scan was Phase-2-era.
    RUNTIME FACT exposed by it: under EL1-direct boot (-cpu max, no
    EL2), QEMU does NOT intercept PSCI SMC — smc #0 at EL1 traps
    UNDEFINED (EC=0x00, ESR=0x02000000 with stale FAR_EL1=0x50000000
    from the Phase-3 tag-4 fault — EC=0x00 does not set FAR). The
    supported config is EL2 boot (virtualization=on); gate updated,
    negatives now panic/FAIL/MISMATCH/FATAL/unexpected/FAILCLOSED.

## Verification discipline notes
- The gate's payload cross-check verifies the GATE'S OWN word list
  against clang — it does NOT extract PAYLOAD2 from phase5.rs. The
  words were consistent, but the class of check has a blind spot;
  runtime is what proved the payloads end-to-end.
- QEMU monitor: HMP needs quoted filenames:
  pmemsave 0x40000000 0x20000000 "/tmp/ram.bin" over
  -monitor unix:/tmp/mon,server,nowait (use nc -U).
- Canonical Lane A command (gate):
  qemu-system-aarch64 -M virt,gic-version=3,virtualization=on \
    -cpu max -smp 2 -m 512M -nographic \
    -kernel target/aarch64-sex/release/sex-kernel-arm64
