# Sequential Spindle desktop work — 29 October 2026

The user requested viewport-aware Spindle rendering followed by at least seven more sequential implementation rounds, then a commit. The previously completed durable-history work remains in this checkout and will be included in that authorized commit. All eight implementation rounds and their final regressions are complete. No Astra agents were used.

Every round preserves GLOBAL_VAS, BOOT-only capability genesis, authenticated sex-pdx edges, fixed incarnation identities and service-owned leases. No Unix process/address-space model is introduced. The architecture constitution remains unchanged. Historical reports and their evidence snapshots remain historical.

| Round | Implementation | Status |
|---|---|---|
| 1 | Authenticated viewport notices, bounded layout, prompt in the last visible row, native resize/recall proof | Complete |
| 2 | Cursor-aware horizontal command window and visible editing position in narrow tiles | Complete |
| 3 | Scrollback paging based on visible rows, stable history/input through resize and paging | Complete |
| 4 | Nonblocking bounded Spindle display delivery so input and Close remain responsive under backpressure | Complete |
| 5 | Bounded history operation deadlines and honest uncertain/timeout status | Complete |
| 6 | Incarnation-owned cancellation that retires pending history transactions and quarantines late receipts | Complete |
| 7 | Explicit safe history retry/recovery and readable operation status | Complete |
| 8 | Desktop integration of pending/recovery states and lifecycle regression; final report and commit | Complete |

Each next round follows the preceding round's remaining gap. Implementation details may change when source or guest evidence requires it; rounds must deliver behavior, not merely rename tests or reports. Focused host tests accompany each change; normal sealed builds and actual guest input/framebuffer evidence verify desktop behavior. Physical controller power-loss, general reserve replenishment, SMP and full epoch replay remain outside these eight bounded desktop rounds.

## Round 1 — viewport-aware terminal

The shell sends bounded layout data through the existing BOOT routes to exact domain/task12 or17. The app accepts only actual sender3, its own immutable incarnation and a newer nonwrapping layout sequence. One desired notice per subject survives queue pressure, waits for frame/chrome admission and is fenced on retirement. The main receive loop keeps pending layout work progressing. No new capability, syscall, mapping or pointer IPC was introduced.

One shared layout calculation fits the four existing catalog-owned surfaces. Explicit-newline text uses only fully visible glyph rows/columns; the last logical row is always the prompt. At minimum height the upper pair collapses and the lower pair uses the body. Shrinking does not change the command, history or scrollback; expanding reconstructs retained output. Diagnostics use bounded metadata and48-byte chunks to fit the existing256-byte atomic serial-record limit.

The normal sealed build is GREEN_MASTER after a90-second probe. All38 host gates,496 Rust test executions and255 Python tests pass. The actual guest loaded the durable four-entry history through a fresh owned handle, recalled `echo spindle17` in the1280×134 startup tile, edited it visibly, expanded to1280×670 and restored the small tile without losing input. All four stages passed exact production-font glyph checks, including unused cells that could retain stale ink. The durable journal remained unchanged.

[Small tile](assets/sexos-spindle-desktop-r1-small-recalled-2026-10-29.png) · [Expanded](assets/sexos-spindle-desktop-r1-expanded-2026-10-29.png) · [Restored](assets/sexos-spindle-desktop-r1-small-restored-2026-10-29.png) · [Historical source/artifact bindings](assets/sexos-spindle-desktop-r1-2026-10-29-evidence.json).

Next: the horizontal window currently follows the command tail. Cursor movement inside a long command needs its own visible window and caret; this is round2.

## Round 2 — cursor-aware editing

Long commands now keep the cursor within the visible command window. The `|` caret and left/right overflow marks are presentation only; they never enter command bytes or durable history. Left/Right and Home/End route through the existing shell edge in insert and normal modes. Backspace in the middle of a line now moves the cursor back to the deletion position.

The sealed90-second build and all38 host gates pass (501 Rust test executions). Production editor tests cover middle insertion/deletion, bounds and normal-mode movement; the formatter checks every cursor position at every command length through256 bytes and widths8–40 columns. The native guest passed nine pixel checkpoints, including a62-character line at the tail and Home, middle insertion/backspace, and expansion with the caret retained. The journal stayed unchanged. [Tail](assets/sexos-spindle-desktop-r2-long-tail-2026-10-29.png) · [Home](assets/sexos-spindle-desktop-r2-long-home-2026-10-29.png) · [Middle deletion](assets/sexos-spindle-desktop-r2-middle-backspace-2026-10-29.png) · [Historical evidence](assets/sexos-spindle-desktop-r2-2026-10-29-evidence.json).

Next: Page Up/Down still uses a fixed eleven-row step. It must use visible output rows, preserve a scrolled anchor when new output arrives and address retained ring entries correctly even after the lifetime line counter saturates. This is round3.

## Round 3 — scrollback follows the viewport

Page Up/Down now uses the visible output-row count while keeping the prompt visible. The retained-ring lookup uses the write position, so wrapping or saturating the lifetime line counter cannot freeze or expose stale slots. New output preserves the selected history by increasing its distance from the tail; resize and eviction clamp selection to retained output. Live-tail selection stays live.

All38 host gates pass (506 Rust test executions), as does the normal90-second sealed image. Production ring tests cover wrap, counter saturation, anchoring, bounds and resize. The native guest paged one row at a time in the small tile through `TYPE HELP` and `SPINDLE TERMINAL`, returned to live output, then expanded and verified the eleven-row step. The prompt and journal were preserved, with five exact-pixel checkpoints. [One row back](assets/sexos-spindle-desktop-r3-page-one-2026-10-29.png) · [Oldest retained output](assets/sexos-spindle-desktop-r3-page-oldest-2026-10-29.png) · [Expanded](assets/sexos-spindle-desktop-r3-page-expanded-2026-10-29.png) · [Historical evidence](assets/sexos-spindle-desktop-r3-2026-10-29-evidence.json).

Next: display submission still retries inline on a full queue. Round4 moves it to a bounded, nonblocking outbox so HID and Close remain serviceable during display backpressure.

## Round 4 — bounded display delivery

A fixed four-pane outbox keeps an immutable active batch and the latest desired text. Each app turn attempts at most four operations before returning to receive. Full preserves the exact operation; permanent denial fences the outbox. Close scrubs queued text before task exit. Registration is also pumped without an inline retry loop. Whole-batch admission is recorded separately from actual compositor presentation.

All38 host gates pass (512 Rust executions), with injected Full, coalescing, ordering and cancellation tests. The normal90-second image is GREEN_MASTER. The guest passed five pixel stages, processed HID with display work pending, then closed the real task and reclaimed its Sext pool. Native evidence proves pending-work responsiveness; injected host tests prove Full behavior. [Burst edit](assets/sexos-spindle-desktop-r4-burst-edited-2026-10-29.png) · [Evidence](assets/sexos-spindle-desktop-r4-2026-10-29-evidence.json).

Next: bound history waits and quarantine unknown completion, including a frozen platform clock.

## Round 5 — bounded history waits

The client now bounds stalled and total cooperative turns as well as observed kernel ticks. Valid receipts reset only the stall budget. A frozen counter still terminates a stalled operation; a regressing counter fails closed. Timeout quarantines late replies, retains RAM history, blocks new persistence and synchronous storage probes, and reports uncertain completion. It never infers rollback from a missing commit receipt. The uncertain state sleeps normally until input arrives.

The90-second sealed image and all38 host gates pass (515 Rust executions). New tests inject full admission, lost open/commit receipts, wrong sequences and frozen/regressing clocks. A real guest boot still loads through a fresh owned handle and visibly recalls the recovered command, without rewriting the journal. [Evidence](assets/sexos-spindle-desktop-r5-2026-10-29-evidence.json).

Next: obtain an explicit own-session quiescence receipt before allowing recovery.

## Round 6 — own-session quiescence

ABORT is bounded data on the existing storage route. A newer own-session sequence may supersede an undelivered receipt, close the service-known lease and scrub staged bytes. No caller-selected handle or name is accepted. Duplicate receipts replay; an old cancellation cannot close a newer lease. The client ignores earlier replies and waits for quiescence before leaving cancellation. Refusal remains uncertain. A completed durable commit survives cancellation.

The90-second image and all38 host gates pass (519 Rust executions). Production client/service tests cover pending open cancellation, cross-subject isolation, terminal subjects, duplicate/stale cancellation, late reply quarantine, fresh leases and retained committed data. The native boot/load/recall regression passes; injected cancellation proof is host-scoped in this round. [Evidence](assets/sexos-spindle-desktop-r6-2026-10-29-evidence.json).

Next: expose explicit status/cancel/retry commands, exclude them from saved history and exercise cancellation and a fresh load in the guest.

## Round 7 — explicit recovery controls

`history-status` reports the operation state. `history-cancel` requests own-session quiescence; `history-retry` is rejected until its receipt arrives. Retry uses a fresh own lease and current RAM history or a newly requested load, with revision checks preserved. Automatic dirty work is suppressed during cancellation and uncertainty. All three controls, plus save/load, are excluded from saved command history.

All38 host gates pass (521 Rust executions), as does the90-second sealed image. The real guest rejected premature retry, cancelled, reported quiescence, retried a load with a different nonzero handle and visibly recalled the same newest command. The journal and four-entry count stayed unchanged. Host tests also cover uncertain save recovery with newer RAM edits and late load receipts. [Fresh-load recall](assets/sexos-spindle-desktop-r7-retried-recalled-2026-10-29.png) · [Evidence](assets/sexos-spindle-desktop-r7-2026-10-29-evidence.json).

Next: expose authenticated history state in the launcher, acknowledge visible lower children when upper panes collapse, then run the full native lifecycle and two-boot durable regression before committing.

## Round 8 — authenticated desktop state and final regression

Spindle publishes a bounded history-state enum through its existing BOOT shell edge. Full retains the latest desired notice. The shell requires the actual sender, exact immutable domain/task/surface/coordinator catalog, LIVE state and a newer nonwrapping sequence. A verified viewport refresh republishes current state after startup registration. Close fences publication; closing, dead and exhausted launcher authority overrides stored status. A history label never substitutes for a pixel-presentation receipt.

The launcher shows loading, saving, cancelling, uncertain, denied or quiescent history state while retaining focus availability for a live app. Actual winning pixels from any exact owned Spindle child can acknowledge that group's presentation. This also handles layouts whose upper panes collapse; foreign children and shell frames cannot supply that bit.

The guest rejected premature retry, completed a real own-session cancellation, rendered HISTORY QUIESCENT / RETRY in the launcher, then loaded through a fresh handle and visibly recalled the newest command. Production-font checks verified the launcher and terminal glyphs. The durable four-entry journal remained unchanged by these control commands.

The final sealed image is GREEN_MASTER after a 90-second probe. All 38 host gates pass, with 526 Rust test executions and 255 Python tests. The full native desktop regression passed Quil and Spindle relaunch, successor input, history save/load, Bell/Linen routing, all five incarnation cleanups and exhausted-reserve launcher glyphs. It retired and zeroed 291 private pages, with all five service acknowledgements joined before reclamation. The guest completed 15 actual NVMe FLUSH operations. Linen processed selection input while its refresh was pending and preserved selection through visible minimize/restore.

A second boot used the identical ISO and a copy of the first boot's committed disk. BOOT-authorized recovery rebound saved task17 data into task12, acquired a fresh owner-checked lease, validated the complete snapshot and visibly recalled echo spindle17. Read-only recovery preserved the journal.

Final ISO SHA-256: 091a2b09bd588da8be38498d5b3eda549357bfa1982931a76d0c18bd6210ce17. The architecture constitution retains SHA-256 4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7. [Final current-source and artifact evidence](assets/sexos-spindle-desktop-final-2026-10-29-evidence.json) · [Selected native trace](assets/sexos-spindle-desktop-final-2026-10-29-trace.json) · [Round 8 bindings](assets/sexos-spindle-desktop-r8-2026-10-29-evidence.json).

[Quiescent launcher](assets/sexos-spindle-desktop-final-launcher-quiescent-2026-10-29.png) · [Successor app](assets/sexos-spindle-desktop-final-spindle-successor-2026-10-29.png) · [Exhausted launcher](assets/sexos-spindle-desktop-final-launcher-exhausted-2026-10-29.png) · [Restored Linen](assets/sexos-spindle-desktop-final-linen-restored-2026-10-29.png) · [Reboot recall](assets/sexos-spindle-desktop-final-reboot-recall-2026-10-29.png).

## Practical limits and next task

These are scoped host tests and real QEMU input/framebuffer evidence. Full and timeout faults are injected in production-state host tests; the native run proves ordinary pending-work responsiveness and real cleanup. There is no physical-controller power-loss, hard real-time, SMP or whole-system epoch-replay claim. The architectural inventory remains 7 open and 23 partial obligations. Relaunch remains limited to the finite BOOT reserve. GLOBAL_VAS, BOOT-only capability genesis and incarnation-owned leases remain the authority model.

The next task is to replace Spindle's legacy synchronous diagnostic storage helpers with bounded typed exchanges in the main receive loop, preserving the history uncertainty fence and allowing HID/Close during those probes. After that, apply bounded delivery to the remaining app paint paths, then build durable document open/save around explicit selected-object authority and improve desktop layout and typography. Further relaunch capacity needs explicit BOOT provisioning with fresh immutable identities; terminal identities and their old capabilities must stay retired.
