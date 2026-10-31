# BOOT-held status-bar civil clock

31 October 2026. The normal status bar now displays validated q35 UTC snapshots
instead of advancing a demonstration clock. It starts with `UTC unknown` and
unknown source/precision. A valid sample shows `UTC HH:MM:SS` and `q35 1s snap`
in the existing 80×22 clock cell at both supported desktop sizes. Source failure,
staleness, rollback and exhaustion remove numeric time. The Browser still offers
the full Gregorian/SexTime/Long Count snapshot panel.

Qualified on `codex/silkbar-civil-time`, based on `876c30be` from
[Browser PR #4](https://github.com/xirtus/sexos/pull/4). The primary checkout and
its running work were not modified. This is an independent named milestone;
it does not consume Claude's numbered primary rounds.

## Authority and data flow

BOOT grants SilkBar PD6/task6 its own typed ClockSource at slot61 with an exact
self-consumer tuple. BOOT checks the role/task identity and empty slot before
granting it. The existing broker PD13/task13 retains its separate Browser
consumer grant. Both use the existing bounded, try-locked q35 read mechanism;
SilkBar performs no raw port I/O, clock writes or runtime capability creation.

One empty outbox is reserved before metadata validation/source work. A full
Display route retains the exact publication and sequence; retries do not sample
again. Source bounds/attempts are reapplied before changing a boot-local
high-water mark. Regressions latch Rollback; the final u32 sequence is reserved
for terminal Exhausted. Neither failure nor Unknown resets the high-water.

The additive kind11 civil update retains the existing 16-byte update layout and
SilkBar wire ABI4. A version/state byte, checked source-epoch seconds and nonzero
sequence have canonical full-word framing. Display authenticates actual IPC
PD6/task6 before decoding; aliases, old sequences and conflicting duplicates
have no effects. The internal model version advances to5. The sealed ABI hash
includes both model source files and the source ABI.

Civil presentation has its own model, separate from all legacy demo/canonical
counter paths. New source observations can refresh health. Duplicates cannot
revive an expired snapshot. Ticks or bounded stagnant/total-turn observations
can hide time, but never advance its civil second. These are observation budgets,
not hard wall-time scheduling bounds. Each visible value remains a last-query
snapshot; delivery cadence and one-second source precision promise no time
authentication or exact update frequency.

The existing wire epoch is preserved. Source/certificate fields are not silently
reinterpreted as SexTime. GLOBAL_VAS supplies addressing, BOOT sanctions genesis,
sex-pdx supplies runtime authority and PKU enforces held authority. This adds no
Unix/Linux/POSIX mechanism or per-process address space.

## Qualification

One frozen B source/checker cohort, sealed GREEN_MASTER:

- ISO SHA-256: `79a48af561534484182f02cb641f38054962996c77f96f619e46a7bf4b9d2b3f`.
- Kernel SHA-256: `7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.
- 574 production inputs and 566 runner inputs match the final source bytes.
- **42** registered host/static checks pass without source drift, including
  **2,628** passing Rust test executions; **880** Python tests pass.
- Retained actual-source controls pass 38 positive executions. Removing the
  publisher guard or occupied-outbox guard independently produces the expected
  failed test exit101. Sources, binaries, compiler commands and logs are retained.
- ISO-extracted Browser, Shell, Display, Quil and SilkBar ELFs decode
  **228,757** instructions with zero inspected vector/FP matches. This is a
  five-ELF static inspection, not whole-image or runtime erasure proof.

Ten native cohorts at 1280×800/e1000e and 1024×768/e1000 pass fresh independent
replay: **12 boots, 70 verified captures, 72 raw screenshots**. Pixel verification
has the stated owned-region scope, rather than claiming every desktop pixel:

- Six Clock normal/invalid-year/Close runs: 16 verified Browser captures with
  exact original-tile restoration. The same 16 frames verify the complete
  status-bar clock cell, including foreground and old-ink removal.
- Two connected desktop cohorts: four live/cold boots and 34 verified captures,
  retaining clipboard, Reader/image, exact document Save and recovery checks.
- Two TLS responsiveness cohorts: 20 verified captures, including six whole
  clock cells, live Quil edits/durable saves and Browser Close during held HTTPS.
  Source time progresses across those workflows and continues after Browser
  retirement. Two additional Close screenshots remain observational only.

The 22 clock-cell proofs comprise 14 positive snapshots and 8 unavailable views.
Initial Unknown has actual-function host and native completed-draw-log scope;
staleness, rollback, exhaustion and forced backpressure have actual-function host
scope. Native clock authenticity, physical RTC and persistent reboot floors are
not established.

## Evidence and diagnostics

[Machine evidence](assets/sexos-silkbar-civil-clock-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-silkbar-civil-clock-2026-10-31-raw.tar.gz).
All **1,505** archive members were verified by size and SHA-256. It retains frozen
sources/checkers, build, bootloader inputs, wire traffic, immutable serial
prefixes, original PPMs, compressed seed/terminal disks, raw document snapshots,
host/Python logs, five ELF/disassembly pairs, compiled controls and independent
replay coordinators.

Actual pixel-identical exports show the [compact desktop](assets/sexos-silkbar-civil-clock-2026-10-31-compact.png),
[unavailable source](assets/sexos-silkbar-civil-clock-2026-10-31-unavailable-compact.png)
and [clock after Browser Close](assets/sexos-silkbar-civil-clock-2026-10-31-after-browser-close-compact.png).
Clock captures park the real pointer; no cursor masks or guest pixels are edited.

The first built guest had no faults and already published/admitted/drew civil
snapshots, but its old boot CLOCK_GATE demanded demonstration recv/redraw
markers and returned Yellow. That attempt remains diagnostic. The new gate
requires exact civil source/publication/admission/completed-draw joins and a
live renderer. A fresh build B is Green; the old result was not relabelled.

The kernel source ordinal is global across both holders. Shared validation now
checks every source record, both exact BOOT grants, tuple/profile/range/attempt
fields and contiguous global ordinals before consumer-specific joins. Complete
atomic kernel records can begin inside an interrupted older user serial line;
their complete ending and every marker are still checked. No foreign source
reader is broadly ignored. Browser/TLS/HTTPS checkers retain their exact query,
service, wire and host-time joins. A pre-send publication-intent record supplies
the causality anchor because a receiver can run before the sender's post-send
return diagnostic.

## Remaining work

This image retains **2 Open / 28 Partial / System Proven false**. Global epoch
commit and deterministic replay remain Open. UiIR is still partial transport;
this feature does not confer rendering authority through stream contents.
The existing built-in clock popup is not a new calendar app or local-time UI.
Physical/SMP, public web/CA, crash/power-loss, hostile containment and a general
daily driver remain separate work.

Next: run the compact 1800-second sustained alpha workflow on this frozen image,
then repeat the wide profile. Measure input/repaint behavior with the new source
traffic; coalesce identical-second presentation without reviving stale data or
losing causal receipts. Continue physical-profile, storage recovery and
lifetime/reserve work from the main OS plan.
