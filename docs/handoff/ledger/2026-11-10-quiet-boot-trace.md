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
