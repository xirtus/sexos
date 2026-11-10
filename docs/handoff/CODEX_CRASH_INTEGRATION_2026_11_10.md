# Crash diagnostics integration (2026-11-10)

Andreas asked Codex to finish a paused workstream before Claude returns.
This integrates PR #78 at `44b6646c926286e36ebd07e6d744017cdcfdcdc0`
with master at `91f00d835487afbb8c82c13846649b909a322fdd` in an isolated
worktree (`codex/finish-crash-diagnostics`).

## Changes beyond Claude's feature

- Resolved the syscall conflict by retaining master’s `wait_unless_pending`
  handshake and exposing `validate_user_bytes` as `pub(crate)` for the crash
  record syscall. Neither side's behaviour was discarded.
- Recomputed the ABI hash from the merged `ABI_INPUTS`, including the new
  crash record module and master's device/tenancy changes.
- Excluded syscall 100 (`PDX_CALL_WAIT`) from the fuzzer. It was unimplemented
  on the feature's earlier base, but master implements a send followed by a
  potentially unbounded wait. Random calls must not park before the fuzzer
  reports completion. Syscall 101 is already outside its generated range.

The remaining feature limits are in
`BOOT_LOG_AND_CRASH_RECORDS_2026_11_09.md`: Display/Shell trigger a recorded
restart rather than restoring the desktop in place, and stop-screen keyboard
handling still needs PS/2 or firmware legacy emulation. This integration does
not claim to finish those follow-ups.

## Validation

- `bash scripts/crash_log_host_gate.sh`: two tests passed.
- Full Python host suite: 1,139 tests passed in 69.170 seconds.
- Merged ABI hash independently verified using `sha256sum`.
- Architectural host checks were launched locally and were still running at
  publication (display pixel tests); CI repeats them and must pass.
- Full build, normal boot, persistence, IPC fuzz and crash-screen warm/cold
  reboot gates: the updated PR's GitHub CI must pass before merging.

No other Claude branch is changed by this integration.
