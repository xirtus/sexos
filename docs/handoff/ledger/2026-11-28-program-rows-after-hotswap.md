# Program windows after a Shell hot swap

Date: 2026-11-28 · Thread: "Remove the open program window limit"

## Symptom (x17, tether session 2026-11-28)

After `@HOTSWAP silk-shell`, every `run NAME` (Tether, anything) answered
"All program windows are in use. Close one first." with only Spindle open.

## Root cause

A hot swap maps a fresh image (`service_restart::apply_swap`), so the pages a
service asked the kernel to keep (`crash_log::keep`, OP_KEEP) came back zeroed.
Shell's restricted-helper journal (`restricted_pool`) then failed closed:
`Ledger::quarantine_all`, `ready=false`, `normal()=false`, so `program_rows()`
offered no row at all. Same loss hit Shell's session page and the restricted
journals in sexnet, sexbell, sexfiles and sexdisplay on their swaps.

## Fix

- Kernel `service_restart`: each service's kept frames are remembered in the order
  it asked (`kept`). A swap moves them to `carried`; the new image's own keep
  calls, in the same order, get their contents copied in. Crash restarts are
  unchanged (frames stay mapped). Host test in `tests/server_hotswap.rs`.
- Shell `restricted_pool::fresh_when_lost`: if the journal is still lost, a
  program row that never ran this boot (activation READY) can still take an
  ordinary program. Rows that ran stay denied; helpers cannot be reserved.
- Spindle's `run` refusal now says "Another program is still preparing" when
  Shell is merely busy, and a real no-free-row refusal logs
  `[shell.program.denied] reason=no_free_row rows=<state per row> pool_normal=
  pool_lost= routes_busy=` (states: 1 ready, 2 active, 3 exhausted, 4 reusable,
  5 rebirthing).
- TOO_LARGE text says 64 MiB (IMAGE_BYTES), not 32.

## Next

Raising the number of program rows past 8 (catalog targets 42-49) is a
separate change: each row is a BOOT domain with its own Shell slots, Display
surface, Files/Net regions and tenancy view, and the catalog is capped at 64
rows by presentation bits.
