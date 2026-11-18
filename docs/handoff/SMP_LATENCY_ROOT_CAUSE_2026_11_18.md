# 4-core latency gate failure: root cause (2026-11-18)

Stream 8 of the kernel redesign (plan PR #405, Phase 4, diagnosis). Gate:
`scripts/latency_gate.py`. Symptom reported in PR #402 and on master: at 4 cores the
gate fails with "busy program 3 never started" (sometimes 4), or loaded p99 ~418 ms.

## Verdict

**Not a scheduler or SMP bug.** The `run bench spin 600` command for busy program
*N+1* is typed into busy program *N*'s window instead of Spindle. Program *N*'s
window takes keyboard focus when Shell maps it, and at 4 cores with PKU under TCG
that happens *after* `[bench.spin.start]` and after the gate has refocused Spindle
(surface 153), so the next command's keys land in the bench window. Bench ignores
keys; the program is never asked for; the gate waits 240 s and fails.

Serial evidence (failed runs, CI run 44204164839):

- Run that failed at "busy program 3": every key of the third command line is logged
  as `[shell.program.key.up] pd=43 task=43 code=...` (pd 43 = busy program 2, row
  ordinal 2), ending with Enter (`code=0x1c`). Nothing reaches Spindle.
- Run that failed at "busy program 4": the same keys are logged for `pd=44` (busy
  program 3).
- The 3-core job shows the same pattern (29 keys to pd 43 across two failed runs).

## Where it fails and where it does not

`latency_gate.py` x3 per configuration, GitHub runners (4 vCPU), QEMU TCG, master
9ea2e6a2 (CI runs 44198778076 and 44204164839):

| config | result | loaded p99 (passing runs) |
|---|---|---|
| 2 cores, PKU | 3/3 pass | 97, 97, 102 ms |
| 4 cores, no PKU (`max,-pku,+rdseed`) | 3/3 pass | 93, 103, 94 ms |
| 3 cores, PKU | 1/3 pass (2x "program 3 never started") | 99 ms |
| 4 cores, PKU (gate default CPU) | 0/6 pass: 4x "program 3", 1x "program 4", 1x p99 422 ms | n/a |

Only PKU at 3+ cores loses the race. That fits a timing cause, not a logic one:

- Under TCG every `wrpkru` is expensive (~3k cycles measured earlier, and QEMU drops
  the vCPU's soft TLB on a PKRU write; not re-measured here), so each context switch
  and syscall return with PKU costs far more than without it (`ipc_roundtrip` 1 core:
  58k cycles no-PKU vs 616k PKU on master, PR #402 table).
- At 3+ cores programs leave core 0 (`smp::home_core`: programs on cores 2..N-1,
  shared-key rows on the last spare core, Files pd 11 on core 3 at 4 cores). The new
  program's surface registration therefore crosses cores (program on 2/3 -> Display
  and Shell on core 0) while two to three cores are pegged by spinners, so Shell maps
  and focuses the window seconds later than at 2 cores, where everything is on core 0.

## The one p99 failure

One 4-core PKU run started all four programs and failed with loaded p99 422.1 ms
(40 keys, so p99 is the single slowest key; the other runs at every config sit at
93-103 ms). One slow key of 40 under 4-core PKU TCG is consistent with the cost
above; it was not reproduced in the other runs and is not evidence of starvation.
Real numbers need KVM or hardware (the bench lane in Phase 0).

## Fixes

1. **Gate (done in this PR):** after each `[bench.spin.start] target=T`, wait (up to
   30 s) for `[shell.focus.set] id=T+317` (the program row's surface) before refocusing
   Spindle for the next command, and fail at once with "typed into program N's window"
   if `[shell.program.key.*]` shows up while waiting for a start. Verified on CI at
   4 and 3 cores with PKU (see the PR).
2. **Desktop (product bug, for the desktop shell owner, not WS4):** a window that
   appears seconds after its command steals focus from the terminal the user is still
   typing in. Shell should not move focus to a newly mapped window if the user typed
   into the focused window since the launch (focus-stealing prevention, as on other
   desktops); mark the new window as wanting attention instead.
3. **No scheduler change is needed for this failure,** so Phase 4's SMP work does not
   have to wait on it. Phase 4 should still revisit placement: at 4 cores core 3 holds
   Files plus every shared-key program row, so programs 2-4 of this gate all share a
   core with Files while core 2 holds only Spindle and program 1.

## Files touched

- `scripts/latency_gate.py`: the wait and the clear failure message.
- No kernel lines.
