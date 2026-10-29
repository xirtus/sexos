# Sustained alpha candidate and desktop continuation

29 October 2026. Round 18 passes, and the first **33 of the requested 100 additional rounds** are complete: rounds 19–51. Continue at **52**, through 118; the [live ledger](ALPHA_100_ROUND_CONTINUATION_2026_10_29.md) remains active. These are substantive implementation/acceptance slices, not a count of test invocations. Preserve one GLOBAL_VAS, BOOT_DAG capability genesis, sex-pdx authority and terminal incarnation identities. One focused Astra review, with a short follow-up, found two text-lifetime defects that were fixed and tested. No Unix process or file-descriptor model was introduced.

## Round 18 passes on its exact historical image

A clean native session completed **1,828.6647 seconds and 29 active mixed-app cycles**. It used actual keyboard and USB-tablet input, exact text/caret glyph checks, two notes, immutable saves with a newer dirty edit, Linen refresh, Spindle storage/history commands, focus/zoom/minimize/restore, pending-I/O Close and reserved fresh subjects 16/17. All five service cleanup roles completed before Sext zero/reuse of **299 private pages**. Strict validation reports no dropped request, diagnostic overflow or fatal fault.

`uialpha` retains 10 exact bytes, hash `0xe41184b5fedbdf93`, identity 393217, generation 1. `uibeta` retains 41 exact bytes, hash `0x9c1d31f9236ee7a2`, identity 458753, generation 32. Both reopen byte-for-byte after booting the **same image** against a copied disk. Spindle's same-image reboot recovers the exact generation-65, source-17 history payload, with fresh private handles and actual `echo alpha17` recall glyphs. The recovered payload contains 64 entries; the existing in-memory history limit is 128 entries.

Image SHA-256: `27b441c4f5b3a4b28fe57267a01f592089a78113a1337896f0aa850ecd15229c`. Final disk SHA-256: `cfc9bb75b2c5974644c5852502cf964b04228f547138361d328ca33d0e5ab79b`. See the [complete binding and reboot results](assets/sexos-alpha-r18-2026-10-29-evidence.json), [compressed complete serial trace](assets/sexos-alpha-r18-2026-10-29-serial.gz) and [compressed exact disk](assets/sexos-alpha-r18-2026-10-29-disk.gz). Rehydrating these committed assets into a fresh directory and rerunning the strict session checker also passes.

The original 1,824.85-second / 32-cycle functional run **failed** strict acceptance: 179 duplicate kernel exit hints overflowed SexFiles' ordinary deferred queue. Its cleanup and bytes worked, but it is not relabeled successful. Another candidate stopped before the timed phase because a pointer receipt interleaved on serial. Both defects were fixed before the clean run above.

Only the first UI captures and final per-cycle PPMs remain available for independent pixel replay. Every cycle's live glyph-check result is recorded; earlier cycle PPMs were overwritten. Full source/image/kernel/checker snapshots and captures are retained locally under `.evidence/alpha-2026-10-29/r18-frozen` and `r18-session`.

## Storage, queue ownership and progress

Live commits use a cooperative generation-bound QJ02 cursor: at most one modeled read, write or FLUSH per journal step. SexFiles drains incoming work first, checks the selected object's identity on each turn, retains completion under Full, and cancels the exiting subject's job before its cleanup acknowledgment. Cut/tear tests recover only a complete previous or committed replacement. Failed or cancelled mutations can already have committed; no rollback or automatic mutation replay is promised.

Initial incremental growth exhausted the fixed extent budget in a failed native candidate. Growth now reserves the complete required run once, zeroes one 4 KiB block per turn, flushes, and orders descriptor/manifest publication with readback and barriers. Cancellation frees only definitely unpublished reservations. Once descriptor publication may have occurred, blocks remain quarantined rather than being reused unsafely. A live transaction also locks its existing workspace object against legacy/probe mutation.

Complete 512-byte replacements skip unnecessary preliminary reads while partial writes preserve neighboring bytes and actual errors. Typed NVMe writes support a whole aligned 4 KiB block only within the already authorized V4 pool, using the same permanent DMA lease. Metadata, legacy/proof and history restrictions remain separate. Cold zeroing uses 16 block commands instead of 128 sector commands; this is an eightfold command-count reduction, not a wall-clock guarantee. Repeated successful I/O diagnostics now share finite budgets; device failures, timeout/quarantine and actual FLUSH receipts remain visible.

Five terminal exit hints have their own bounded retention beside eight ordinary deferred requests. Exact duplicates coalesce. Only a correctly decoded kernel-sender hint enters that lane; the hint itself grants no authority. Services still attest terminal custody through existing grants.

Individual block completions are still synchronously awaited inside service turns. Fully retained asynchronous block completions, power-loss behavior beyond the modeled allocator/atomic-sector/FLUSH assumptions and arbitrary physical-device failures remain open.

## Desktop text and lifecycle

Quil, Linen and the launcher publish bounded immutable text transactions over existing BOOT routes. Display stages partial chunks privately, validates complete coverage/hash/sequence, then replaces text once. Incomplete uploads leave the previous view visible. Actual sender, live incarnation and surface ownership govern admission; a surface number grants no authority. Register payloads contain text data and identifiers, never borrowed addresses.

The client retains one active replacement and one coalesced latest view. Application acknowledgments bind actual display sender, surface and sequence. They confirm text-state application, not pixels. Queue Full retains exact requests and receipts. The legacy reply syscall returns a generic nonzero result for a full reply slot; an intervening wide native run exposed lost acknowledgment after paint 38. Display now retains every nonzero reply result, retries once per pass, and cancels it only on admission or authoritative surface/app cleanup. This retries a receipt, never a mutation.

Review found an ACK → new queue → poll ordering bug that could resurrect an older queued view and stall future output. Queueing now discards that superseded view and rejects non-increasing paints. It also found staging surviving ordinary surface destruction: destruction, vacant rebinding and terminal removal now scrub partial bytes and pending replies, including removed coordinator-owned frames. Exact regressions exercise both sequences.

The compositor caches winning surfaces once per scanline span. Text-only redraws coalesce the previous/new glyph footprints; geometry, focus and cleanup retain full-scene redraws. The damage path restores the cursor background first, composites actual winning pixels, then captures/redraws the cursor. Host tests compare actual compositor/damage-loop pixels for clipping, overlap, focus, launcher overlays, old-line erasure and hidden parents. Host framebuffer/overlay primitives are sentinels; native glyph checks remain required.

Alt+Tab switches live app frames globally, including leaving Spindle. Plain Tab stays local to Spindle, with modifier/release ownership retained across focus changes. Uncreated legacy placeholders no longer occupy normal tiles or the focus cycle. Cursor submission retains the latest coordinates under display queue Full; counters advance only after admission. Pointer normalization receipts are atomic.

BOOT activation now binds both exact subjects/tasks **and** explicit immutable family/adjacent-ordinal records. Cross-family, skipped and wrapping ordinals fail before restoring any leaf. This strengthens activation authority; it does **not** add another reserved app. At that checkpoint each family still had only its original and one successor; round 45 adds the third Quil subject below.

## Latest integrated candidate: rounds 19–43

Image SHA-256: `31950b30d4b51996d25bf745bac3b4b3c35b12b76340d1ad607d31c68dd9d9a6`. Sealed BUILD PASS / GREEN_MASTER for 90 seconds; **38 architectural gates, 611 Rust test executions and 275 Python tests pass**. Architecture text is unchanged, SHA-256 `4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`.

Both 1280×800 and 1024×768 native runs pass the complete document walkthrough, twenty alternating one/five-key bursts (**60 actual characters** per resolution), exact latest text/caret glyphs, final durable payloads, one mixed-app rehearsal, pending-I/O Close, reserved successors and cleanup. Same-image document and history reboots pass. Global Alt+Tab, returned exact Spindle prompt, app-local Tab, absence of uncreated placeholders and all five Spindle cleanup roles also pass at both sizes.

| Host-observed burst action → actual glyph pixels | 1280×800 | 1024×768 |
|---|---:|---:|
| Median | 0.201 s | 0.173 s |
| 95th percentile | 0.296 s | 0.234 s |
| Maximum, including first post-restore sample | 4.076 s | 3.511 s |

For context, the older sustained image recorded a 19.50-second median input-to-glyph observation (23.93-second 95th percentile). Its repeated reflow/mixed-workload conditions differ from the latest burst test; do not infer a controlled speedup ratio.

The final mixed-cycle single-key glyph observations are 0.236 / 0.230 seconds. Cold saves are 8.54 / 10.60 seconds wide and 8.32 / 8.09 compact. Warm Save receipts remain about 4.90 / 4.67 seconds. The mixed Save-to-pixel observations, **including concurrent Linen/controller actions**, are 12.08 / 9.33 seconds. These are QEMU/TCG host observations with polling, framebuffer capture and concurrent guests; neither hard real-time bounds nor controlled comparative speedups are established. Initial restore outliers and storage/menu latency remain blockers.

See the [latest complete source/image/test/run binding](assets/sexos-alpha-r43-2026-10-29-evidence.json), [wide capture](assets/sexos-alpha-r43-wide-2026-10-29.png) and [compact capture](assets/sexos-alpha-r43-compact-2026-10-29.png). Complete compressed traces and exact disks for every final run sit alongside that binding. Full local immutable artifacts are under `.evidence/alpha-2026-10-29/r43-*`.

The round-43 image now also passes its own strict sustained gate: **1,819.6704 seconds, 32 active mixed-app cycles and 299 private pages zeroed/reused**, with no dropped request, diagnostic overflow or fatal fault. Same-image document and Spindle history reboot gates pass. The final notes are `uialpha` (10 bytes, generation 1, hash `0xe41184b5fedbdf93`) and `uibeta` (44 bytes, generation 35, hash `0x2898934531bd2c7b`). Final disk SHA-256: `0858ad31ec32e88650d76db282f59fbd3c00c4f3949ceeec89f00f57c5f90961`. See the [sustained binding, complete results and both reboot results](assets/sexos-alpha-r43-sustained-2026-10-29-evidence.json); complete compressed serial/disk assets sit alongside it, with full local captures under `.evidence/alpha-2026-10-29/r43-sustained*`.

The mixed workload still exposes severe latency: median input-to-exact-glyph observation **19.722 seconds**, 95th percentile 24.285, maximum 25.241. Warm Save receipts have median 4.590 seconds, while Save-to-pixel including concurrent Linen actions has median 13.289. This sustained workload includes repeated focus/reflow and concurrent guests and differs from the burst test above. The short burst result cannot stand in for mixed desktop responsiveness; this remains a measured alpha blocker.

The strict checker was not relaxed. The source freezer verifies a sealed GREEN build, unchanged production inputs and exact ISO/kernel hashes, copies recorded production/checker/host-test sources, rejects outside-root symlinks and publishes its binding last. A failed build cannot boot an old ISO.

## Round 44: automatic document-list freshness

After a successful Save, Quil sends a bounded coalesced invalidation over its existing BOOT shell route. The shell validates the actual sender, catalog task and exact empty payload, then checks live lifecycle authority before requesting a read-only Linen rescan. Queue Full, transient kernel retry and an in-flight snapshot retain the work; terminal cleanup cancels it. The notice carries no object address or authority. Quil focus and Linen selection remain unchanged.

Final image SHA-256: `5436e59028e7fa76495288e9ceb6140d021cb36265dc0b19f6c8c71505a07b6f`. BUILD PASS / GREEN_MASTER (90 seconds), 38 architectural gates, **615 Rust executions and 276 Python tests pass**. Both native resolutions pass Save and Save As with automatic counts six/seven and exact new-name glyphs before any manual rescan, followed by burst typing, pending Close, fresh successors and cleanup. These short runs are rehearsals, not sustained acceptance. See the [complete round-44 binding](assets/sexos-alpha-r44-2026-10-29-evidence.json), [wide list](assets/sexos-alpha-r44-wide-auto-list-2026-10-29.png) and [compact list](assets/sexos-alpha-r44-compact-auto-list-2026-10-29.png). Full immutable artifacts are retained under `.evidence/alpha-2026-10-29/r44-*`.

## Rounds 45–51: finite relaunch and settled desktop publication

BOOT now reserves a second fresh Quil successor: **PD/layout 18, surface 307 and hardware key 9**, with distinct 39-page private backing. Lifecycle slot 54, activation 55, Close 56 and input route 57 exist before seal. The ordered grants are logical Quil 9, incarnation 2 (9 → 16), then incarnation 3 (16 → 18). Six app custody entries fit the kernel, services, display and Sext tables. The required service roles remain exactly five. Activation still requires predecessor death, every service acknowledgment, private-leaf retirement and the existing BSP-only guard. Terminal receipts cannot restore old leaves. This is **two finite Quil reopens**, and Spindle still has one; it is not runtime replenishment.

Document storage has a separate session and completion destination for 18. Exact BOOT catalogs supply task identities throughout document admission, Spindle history/activation and viewport delivery; domain and task numbers are not assumed equal. Host tests deliberately use different numbers. Client-supplied identifiers never replace lifecycle authority or transfer predecessor-private RamFS handles.

Native testing exposed two real UI defects before acceptance. A fresh Quil could open a note while keeping its command palette active, swallowing ordinary typing as a palette command. A recreated display slot could also scrub paint 1 while Quil kept waiting for its acknowledgment. Successful current-revision Open now enters editing. A fresh authenticated viewport notice discards obsolete text uploads and starts a strictly newer complete paint; old acknowledgments cannot complete it. This does not replay storage mutations.

The shell retains frame and child creation across Full, checks the exact live parent and current bounds, and waits for create/chrome admission before publishing a viewport notice. Its sixteen fixed presentation-state slots cover seven BOOT frames and nine child panes; this expands retained metadata, not authority or the display's sixteen-surface limit. Close/minimize cancel invalid pending creates; hard rejection never becomes readiness. Same-size restore produces a new viewport sequence.

Fill rectangles now damage only their old/new footprints, coalesced with text damage in a drain. Identical fills request no repaint; empty fills actually remove previous rectangles. Actual compositor tests compare every pixel against a full composite across move/recolor/clear, overlap, clipping, focus and hidden parents. Native scenes retain the existing structural redraw path for geometry/focus/cleanup.

Final image SHA-256: **`74554074e2f316a284a816fc7b4646bd50895b6ca37a6c9dfcff5d801b4a184f`**. BUILD PASS / GREEN_MASTER (90 seconds), **38 gates from frozen source, 633 Rust test executions and 285 Python tests**. The frozen host run reports no source drift. The freezer includes the declarative proof inventory and its model input fixture; these remain host inputs, not native evidence. A host gate that observes edited, added or removed source now fails rather than inheriting mixed-checkout success.

Both 1280×800 and 1024×768 final native rehearsals pass automatic list freshness, 20 typing bursts / 60 real characters, immutable Save with a newer dirty edit, resize/minimize/restore, pending-I/O Close, exact document recovery in 16 and 18, a real edit/save in 18, finite exhaustion, Spindle 17 and all six app cleanups. Sext zeroes/reuses **338 private pages**. Same-image document and history reboots also pass. The final pixel checker waits for application acknowledgment and verifies the entire visible editor for both exact current glyphs and erased old rows; a mid-repaint image cannot pass merely because the new lines are visible. The acknowledgment itself is still not called a pixel receipt.

| Final short-run host observations | 1280×800 | 1024×768 |
|---|---:|---:|
| Burst → fully checked editor median | 0.236 s | 0.173 s |
| 95th percentile / maximum | 0.287 / 0.346 s | 0.258 / 0.262 s |
| One mixed-cycle input → checked editor | 0.258 s | 0.264 s |
| Warm Save receipt / pixels including Linen actions | 4.633 / 6.540 s | 4.645 / 5.225 s |
| Two cold Save receipts | 8.849 / 9.343 s | 8.398 / 8.861 s |

These are short QEMU/TCG observations, not hard bounds or a controlled comparison with the preceding sustained workload. Storage remains slow. A **new 30-minute round-51 session is running** against the frozen final checker/image and must pass strict validation and both reboots before that image earns sustained qualification.

See the [complete final binding and results](assets/sexos-alpha-r51-2026-10-29-evidence.json), [wide settled editor](assets/sexos-alpha-r51-wide-2026-10-29.png) and [compact settled editor](assets/sexos-alpha-r51-compact-2026-10-29.png). Full frozen sources/checkers and captures are retained under `.evidence/alpha-2026-10-29/r51-*`; complete compressed traces and exact disks accompany the committed binding. Failed rounds 45/47 (palette and obsolete-upload stalls) and 49 (exhausted geometry-log budget) remain explicit failures in the binding and compressed traces. The latter was corrected with incarnation-specific viewport publication rather than relaxed geometry or glyph checks.

## Continue at round 52

**67 substantive rounds remain** under the active request. First finish the final image's sustained qualification. Next make original/fresh app presentation coherent and show the actual finite reserve without turning labels into authority. The new Quil surfaces still inherit legacy standalone styling and generic app indicators; they should retain the logical app's appearance and name.

Then attack measured storage and repeated-reflow latency. Document payloads still use bounded register messages and serial round trips; block completions still synchronously wait within a service turn. Investigate bounded pipelining and retained correlated completions on existing BOOT routes, preserving immutable staging, Full retention, shared-buffer custody, DMA uncertainty quarantine and terminal cleanup. Do not introduce borrowed pointers, ambient handles, automatic mutation retry or new runtime capability creation to hide latency.

The target remains a trusted-builtins QEMU alpha. Open limits include finite relaunch reserve, twelve 64 KiB workspace journal slots, 12 KiB ASCII notes, 128-entry history, offline cached Kaleido content, real networking/DNS/HTTP/TLS, named physical hardware, hostile-code containment, complete epoch replay and physical SMP. The architecture inventory remains seven open and twenty-three partial obligations. No alpha tag or general daily-driver claim is made by this batch.
