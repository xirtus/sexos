# 2026-11-28: Files, Display and Shell restart when stuck, never when only busy

Safe version of Gemini's widened watchdog (#821/03f624cd, taken back out): it gave
Files, Display and Shell a plain 60 s "held a message" limit and called them services
that answer every call. Busy Files was then ended mid-work under emulation, programs lost
their Files bindings, and callers were told PEER_RESTARTED for calls these services never
answer.

- `kernel/src/service_restart.rs`: a separate desk watch for Files (11), Display (1) and
  Shell (3). `desk_verdict` says Hung only when all three hold: the service has held a
  message for over 60 s without coming back to its inbox, its own code has made no
  syscall for the whole 60 s, and its task is parked (blocked, no wake pending, on no
  core). Computing (on a core or queued) or parked-but-still-making-syscalls is only Slow:
  one `[service.slow]` line per held message, never ended. Waiting for an answer is never
  holding (every receive and the reply wait mark the inbox visited). Hung goes down the
  existing hang path (`[service.hung] ... desk=1`, crash record "no answer (stuck)",
  normal restart). `hang_limit` and `answers_every_call` are unchanged.
- `kernel/src/syscalls/mod.rs`: syscall entry stamps `service_restart::ran(pd)`; for
  the three desk services only, one store at most per tick into its own cache line.
- `tests/fault_recovery.rs`: desk scope + `desk_verdict` cases.
- `scripts/driver_restart_gate.py` new last round `fileshang`: `crash files hang`
  mid-request; the watch must end Files after >60 s, Files restarts, files read back and
  save; the earlier driver rounds still assert Files never restarted.
- Pins: `kernel/src/service_restart.rs` whole-file pin re-taken; syscalls fn-v1 pin
  unchanged. No ABI change.
