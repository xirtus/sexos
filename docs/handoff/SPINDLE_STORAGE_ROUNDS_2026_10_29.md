# Five sequential Spindle storage rounds — 29 October 2026

Baseline c286d981 was pushed to origin/master before this work. GLOBAL_VAS, BOOT-only capability genesis and sex-pdx authority remain fixed. No agents are used. All five rounds and their final regressions are complete. This report extends the eight completed desktop rounds.

| Round | Deliverable | Status |
|---|---|---|
| 9 | Correlated nonblocking stat probes, bounded waits and pending receipt delivery | Complete |
| 10 | Coherent staged object listing with generation conflict detection | Complete |
| 11 | Migrate all legacy disk commands to bounded asynchronous jobs | Complete |
| 12 | Explicit cancellation/recovery, object mutation fences and history coordination | Complete |
| 13 | Readable desktop probe state, native input/Close overlap, final regression and alpha handoff | Complete |

Each round freezes its own source/artifact evidence. The final handoff will distinguish QEMU alpha goals from hardware daily-driver readiness and name executable acceptance criteria.

## Round 9

The new register-only exchange uses a distinct reply tag, nonwrapping sequence, exact live BOOT subject and one retained pending receipt. The app attempts one admission per turn, receives only correlated SexFiles replies, bounds stalls even under a frozen clock, quarantines uncertainty and cancels locally before task exit. No input is drained. The existing bridge runs with the original caller identity; object numbers remain service data under the pre-existing BOOT storage route.

All 38 host gates pass (531 Rust executions), as does the sealed 90-second runtime. The native guest returned object0 size4096 and rendered further command input. Full and stalled-reply behavior is host-injected proof, not native fault injection. [Evidence](assets/sexos-spindle-probes-r9-2026-10-29-evidence.json).

## Round 10

The disk listing stages all 15 slots and their complete 24-byte names. It publishes only after the manifest generation matches the starting generation and every name validates. A conflict discards staged data. No partially fetched list appears as a completed snapshot. The native guest enumerated the actual three system objects and continued editing visibly. All 38 host gates and the sealed runtime pass (533 Rust executions). [Evidence](assets/sexos-spindle-probes-r10-2026-10-29-evidence.json).

## Round 11

Every legacy Spindle storage receive/drain helper has been removed. Disk/ls, create/delete/rename, bounded pattern fill, truncate and binary verification now use typed jobs with one admission attempt per main-loop turn. Service-owned bindings check manifest generation before continuation, and duplicate mutations replay their receipt without re-execution. Mutations require real NVMe flush and session-end receipts before completion. System slots cannot be modified by these diagnostic jobs; names, IDs, lengths and decimal arithmetic are bounded.

Native testing exposed two liveness issues before acceptance: the original probe stall budget was too short for cold extent allocation, and simultaneous background history could time out during a long probe. The accepted version has a larger finite diagnostic budget with explicit deadline records, plus one storage owner at a time. A queued probe gets the next completed-job boundary, dirty history remains in RAM until its turn, and uncertainty fences the other client. Input and Close remain outside both state machines.

All 38 host gates pass (538 Rust executions), as does the sealed runtime. The native guest created a real object, filled and byte-verified 513 binary bytes, truncated and reverified 17 bytes, renamed and deleted it, completed background history saves, then rendered further input. A failed or cancelled diagnostic mutation may have partial effects; this is not a transactional document save API. [Evidence](assets/sexos-spindle-probes-r11-2026-10-29-evidence.json).

## Round 12

Explicit probe-status/cancel/retry commands are excluded from saved command history. A fresh own-session cancellation can supersede an undelivered receipt and retire the service-known binding. Earlier replies are quarantined. Only read-only stat/list/verify jobs can be retried after quiescence; mutating jobs require inspection and a new command. A refused or missing cancellation receipt stays uncertain. Abort does not roll back persisted effects.

All 38 host gates pass (543 Rust executions), as does the sealed runtime. New tests cover pending-BEGIN cancellation, late replies, stale cancellation after a fresh binding, isolation from the other subject, denied cancellation, parser overflow and refusal to replay mutations. The native guest completed binary jobs, rejected premature retry, received quiescence and completed a fresh stat retry. [Evidence](assets/sexos-spindle-probes-r12-2026-10-29-evidence.json).

## Round 13

The launcher now distinguishes active, cancelling, uncertain, denied and quiescent disk jobs through the existing authenticated status route. History uncertainty takes precedence; a terminal lifecycle state still overrides all app data. Listing rows include object IDs, completion text names the object and byte count, and probe-help describes the actual commands and partial-mutation limits. Obsolete help claiming that Spindle lacks real HID or requires runtime spawning was corrected.

Receipt review also found and fixed a backpressure gap for duplicate/stale requests. The service now retains their pending reply separately from its last fresh receipt. A full queue cannot lose the replay or overwrite the receipt needed to replay the latest operation. A production-state test covers both cases.

The final native run completed the 513-byte binary object workflow and cancellation/retry controls, then verified actual launcher glyphs for DISK QUIESCENT / INSPECT. It admitted a new 64 KiB fill, observed a real service binding, typed xyz while the job was unfinished and verified those prompt glyphs. F11 reached Close while the job was pending. Local work was fenced before task finish, SexFiles retired the binding before its cleanup acknowledgement, and Sext zeroed/reclaimed the private backing. No completed-fill receipt was substituted for this overlap proof.

The final normal image is GREEN_MASTER after a 90-second probe. All 38 host gates pass, with 546 Rust test executions and 255 Python tests. The separate full desktop run passed Quil and Spindle relaunch, successor input/save/load, Bell/Linen routes, exhausted launcher states, native glyphs, refresh/input overlap and minimize/restore. All five enrolled incarnations retired 295 private pages with complete service acknowledgements before reclamation. That run completed 15 actual NVMe FLUSH operations.

A second boot of the identical ISO recovered the four-entry successor history through the BOOT recovery grant, acquired a fresh task12-owned lease and visibly recalled echo spindle17. Recovery left the durable journal unchanged.

Final ISO SHA-256: 8fabf635f402f230b5ca7c4c7363c67ca573fce5c46dd1072cfc240bdcf350c2. ARCHITECTURE.md remains unchanged at 4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7.

[Final current-source/artifact evidence](assets/sexos-spindle-probes-final-2026-10-29-evidence.json) · [Selected trace](assets/sexos-spindle-probes-final-2026-10-29-trace.json) · [Round 13 bindings](assets/sexos-spindle-probes-r13-2026-10-29-evidence.json).

[Typing during the admitted job](assets/sexos-spindle-probes-final-pending-input-2026-10-29.png) · [Probe launcher state](assets/sexos-spindle-probes-final-probe-launcher-2026-10-29.png) · [After pending Close](assets/sexos-spindle-probes-final-closed-pending-2026-10-29.png) · [Restored Linen](assets/sexos-spindle-probes-final-linen-restored-2026-10-29.png) · [Reboot recall](assets/sexos-spindle-probes-final-reboot-recall-2026-10-29.png).

## Scope and next session

The existing BOOT storage route retains its established diagnostic scope. Object numbers are data for SexFiles, not newly minted object capabilities or Unix descriptors. These bounded pattern jobs are not a transactional document save API. Failure, timeout or cancellation can leave partial mutations; only a quiescence receipt permits new work, and automatic retry is restricted to reads. Mutations require inspection and a new explicit command.

Full queues and stalled/lost replies are injected in host tests. The guest proves actual ordinary jobs, input during an admitted unfinished job, Close, service retirement and backing reclamation. This is not a hard real-time, physical power-loss, hostile-code isolation, SMP or whole-system epoch proof. The architecture inventory remains 7 open and 23 partial obligations. App relaunch still has a finite BOOT reserve, and normal networking remains disabled.

The next session is specified in [the alpha daily-driver plan](ALPHA_DAILY_DRIVER_NEXT_SESSION_2026_10_29.md): convert Quil's real document paths to nonblocking loading/saving, prove durable two-object recovery, finish readable document UX and run a sustained native alpha session. Sustainable BOOT-sanctioned lifecycles, real networking and named hardware have separate release gates. The plan includes five sequential implementation rounds, exact source areas, exit criteria and a ready-to-use continuation request. No automation or new chat was created.
