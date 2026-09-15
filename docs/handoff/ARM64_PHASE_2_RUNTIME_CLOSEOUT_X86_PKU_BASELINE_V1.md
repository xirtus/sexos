# ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1 — Record (2026-07-21)

Validation-only mission. No source changes. No STOP FIRST authorization was
requested or needed — both runtime lanes skipped for absent QEMU; nothing
failed in a way that would authorize code changes.

## Verdict
- Lane A (ARM64 Phase-2 runtime): **ARM_RUNTIME_SKIP_NO_QEMU**
- Lane B (x86 PKU-on baseline): **X86_PKU_RUNTIME_SKIP_NO_QEMU**
- Overall: **BLOCKED** — ARM64_PHASE_3_MMU_GLOBAL_VAS_V1 remains blocked.
- S1POE status: UNVERIFIED (unchanged; no claim made, none attempted).

## Host and toolchain
- Host: aarch64-apple-darwin, macOS 26.5.2 (Darwin arm64)
- rustc 1.99.0-nightly (0e29c21d9 2026-07-21), LLVM 22.1.8
- cargo 1.99.0-nightly (3efb1f477 2026-07-17)
- qemu-system-aarch64: ABSENT (PATH + /opt/homebrew/bin + /usr/local/bin + /opt/local/bin)
- qemu-system-x86_64: ABSENT (same checks)
- Warning-count note: x86 pin (153 warnings) valid only under the toolchain
  above; re-baseline on any toolchain change (see Phase-2 handoff, Toolchain).

## Reading performed (READ FIRST list)
- docs/handoff/ARM64_PHASE_2_ENTRY_SERIAL_V1.md — present, read
- docs/handoff/ARM_S1POE_AUDIT_V1.md — present, read
- docs/handoff/PDX_SYNCCALL_GODMODE_SEMANTICS_V1.md — present, read
- docs/handoff/X86_BASELINE_MARKERS_V1.md — **ABSENT on this tree**; actual
  canon is docs/handoff/MASTER_RUNTIME_GATE_V1.md (addendum appended there)
- docs/handoff/AGENT_HANDOFF_GP_CLOCK.md — **ABSENT on this tree**; clock
  liveness canon lives in MASTER_RUNTIME_GATE_V1.md (silkbar.clock.send >= 2)
  and SILK_DE_100_SAFE_SEQUENCE_V1.md

## Lane A — ARM64 Phase-2 runtime
- Build: scripts/build_arm64_phase2.sh — GREEN, no source changes required.
- ELF: machine=183 (EM_AARCH64), type=2 (EXEC), entry=0x40080000
  (matches handoff), undefined symbols = 0 (llvm-nm -u).
- Run: scripts/run_arm64_phase2.sh → exit 77, "SKIP_NO_QEMU:
  qemu-system-aarch64 not found on this host".
- Canonical command (unexecuted here):
  qemu-system-aarch64 -M virt,gic-version=3 -cpu max -m 512M -nographic \
    -kernel target/aarch64-sex/release/sex-kernel-arm64
- Ordered markers: NOT observed (no runtime).
- CurrentEL: NOT observed (EL2 expected on QEMU virt; PASS must not be
  hardcoded to EL2).
- EL2→EL1: NOT exercised. DTB magic: NOT observed. UART delivery: NOT
  observed. Terminal WFE: NOT observed.
- Negative scan: nothing to scan.
- Result: **ARM_RUNTIME_SKIP_NO_QEMU**
- Log: logs/arm64_phase2_runtime_closeout.log

## Lane B — x86 PKU-on baseline
- Canonical path discovered (MASTER_RUNTIME_GATE_V1.md):
  qemu-system-x86_64 -M q35 -m 512M -cpu max,+pku -cdrom sexos-v1.0.0.iso \
    -device nec-usb-xhci,id=xhci -device usb-tablet,bus=xhci.0 \
    -drive if=none,id=nvm,file=<repo>/.gate_master/nvme.img,format=raw \
    -device nvme,serial=sexos01,drive=nvm -serial file:$LOG -display none \
    -no-reboot -no-shutdown
  Canonical gate host is Linux (paths reference /home/xirtus_arch/...).
  sexos-v1.0.0.iso is NOT present on this host.
- Build: canonical x86 build already GREEN this session via gate
  (cd kernel && cargo build; 153 warnings pinned toolchain; 0 errors).
  Not rebuilt — no source changes since (token discipline).
- Run with PKU: NOT EXECUTED — qemu-system-x86_64 absent. `-cpu max,+pku`
  neither accepted nor rejected here (no QEMU to test it). No CPU-model
  substitution attempted, per mission rule.
- PKU proof: none — no CPUID result, no CR4.PKE proof, no restricted
  ring-3 PKRU proof. `-cpu max,+pku` on a command line would NOT have
  counted as proof anyway.
- Baseline proof: none — no spawn/scheduler/clock/stability observation.
- Context: last recorded master run (MASTER_RUNTIME_GATE_V1.md) was
  RED_MASTER (ORDER_GATE FAIL) — pre-existing x86 state on the canonical
  Linux gate host, to be re-checked when Lane B actually runs.
- Result: **X86_PKU_RUNTIME_SKIP_NO_QEMU**
- Log: logs/x86_pku_runtime_baseline.log

## Files changed (documentation/logs only)
- NEW: logs/arm64_phase2_runtime_closeout.log
- NEW: logs/x86_pku_runtime_baseline.log
- NEW: docs/handoff/ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1.md (this file)
- EDIT: docs/handoff/ARM64_PHASE_2_ENTRY_SERIAL_V1.md (closeout-attempt record)
- EDIT: docs/handoff/MASTER_RUNTIME_GATE_V1.md (PKU-baseline status addendum)
- Backup before edits: Sex_ArmV9/backups/doc_pre_closeout_20260721/
- Mirrored to Sex_ArmV9/docs/handoff/.
- ZERO source/code changes. Forbidden-file set untouched (gate re-verified).

## Phase-3 status
**ARM64_PHASE_3_BLOCKED.** Unblocks only on ARM_RUNTIME_PASS AND
X86_PKU_RUNTIME_PASS from a host with both QEMU binaries. The prompt at
Sex_ArmV9/prompts/ARM64_PHASE_2_RUNTIME_CLOSEOUT_X86_PKU_BASELINE_V1.prompt
is unchanged and ready to rerun there.

## Gotchas (this mission)
1. Two READ FIRST docs (X86_BASELINE_MARKERS_V1.md, AGENT_HANDOFF_GP_CLOCK.md)
   do not exist on this tree — check existence before trusting prompt
   references; canon is MASTER_RUNTIME_GATE_V1.md.
2. scripts/run_arm64_phase2.sh exits 77 with SKIP_NO_QEMU — scripts and
   agents must treat 77 as SKIP, never as FAIL or PASS.
3. macOS has no GNU timeout — this mission's bounded execution used the
   runner's built-in sleep/kill pattern; keep host-native bounding on macOS.
4. No *.iso at repo root on this host — Lane B on a new host must mint the
   ISO via the canonical path before QEMU; do not improvise a boot flow.
