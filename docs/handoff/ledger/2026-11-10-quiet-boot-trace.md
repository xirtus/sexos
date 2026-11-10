## Quieter boot: successful loader internals are opt-in (2026-11-10)

- Normal images omit per-segment ELF mapping/copy/BSS messages and per-domain loader
  checkpoints. Build with `SEXOS_VERBOSE_BOOT=1 ./scripts/entrypoint_build.sh` to retain
  those details. Values other than exactly `1` keep normal quiet behavior.
- No global log filter: watchdogs, errors, entrypoint PTE assertions, SASOS layout,
  loader/domain completion, structured authority records, `[wifi.*]` and `[sexnet.*]`
  remain on the existing log path. All loads, copies, zeroing, flag updates, checks and
  checkpoint assignments still execute. Quiet trace arguments are not evaluated.
- Evidence: production macro compiled and executed with the variable unset, `0`, and
  `1`; only `1` emits/evaluates trace arguments, ordinary service diagnostics always emit.
  Six kernel-design guardrail tests pass. Release x86_64 kernel builds pass in both
  normal and verbose modes. Independent spot review found no removed-text gate dependency.
- Historical-log estimate only: PR #111 run 42812580544, head `4c1ef5c0`, normal runtime
  serial before `[silkshell.ready]` contained 348,036 UTF-8 bytes; 159,220 (45.7%) match
  success traces changed here. This is a classification of old output, **not** a fresh
  measured boot-time/volume improvement; candidate CI/guest evidence must confirm it.
- Kernel growth: 11 source lines for an opt-in logging macro. This is kernel-internal
  diagnostic mechanism and cannot move to a server; no driver/policy/ABI addition.
  The 21,500 nonblank-line budget remains unchanged and passes.

### Fresh guest comparison

Clean sealed builds of candidate `1b905a59` were booted in QEMU 11.0.2 TCG, one
core, 1280x800, identical blank NVMe disks. Only `SEXOS_VERBOSE_BOOT` differed.
Before `[silkshell.ready]`, normal output was **183,515 UTF-8 bytes / 2,602 lines**;
verbose output was **341,139 bytes / 5,461 lines**: **157,624 fewer bytes (46.2%)**.
The selected loader-detail families emitted zero lines normally and 2,030 with verbose
logging. Both boots reached Shell/app registration and Linen disk publication.

ISO SHA-256: normal `6dffd4a4f222b32a33133ea83b89f3398d73d5a00f64704a265c53bc1b4eb974`;
verbose `0a2671b78d654b5328cf0ca7b8ba8822e1513bd45af8120d6e472fadfdf0e99d`.
Raw serial, source/ISO hashes, QEMU version, desktop dumps and the replay helper are in
`/Users/xmacbookm5/Projects/sexos-evidence/quiet-boot/{normal,verbose}/` and
`quiet-boot/measure_boot.py`. Builds without local FFmpeg/libc++ qualify this log comparison
only. Concurrent host builds invalidate a latency comparison; **no boot-speed claim**.
