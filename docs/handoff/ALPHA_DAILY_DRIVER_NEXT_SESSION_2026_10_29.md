# Next session: shortest path to a useful SexOS alpha

29 October 2026, Europe/Berlin. Read together with [the five storage rounds](SPINDLE_STORAGE_ROUNDS_2026_10_29.md), [the eight desktop rounds](SPINDLE_DESKTOP_ROUNDS_2026_10_29.md), and [the architectural truth inventory](SEXOS_ARCHITECTURAL_TRUTH_2026_10_28.md).

The immediate release target is a useful **QEMU desktop alpha for trusted built-in apps**: write notes in Quil, manage documents in Linen, inspect the system in Spindle, save reliably, and resume work after reboot. A general online or physical-hardware daily driver needs additional work. Treat those as separate exit gates, so a good screenshot or a mock network response cannot move the release label forward.

## Rules and starting evidence

Keep one GLOBAL_VAS. BOOT_DAG alone creates capabilities; sex-pdx decides runtime authority; PKU accelerates enforcement. No per-process tables, Unix processes, file descriptors, ambient paths, runtime capability minting, or reuse of terminal domain identities. Reuse the register-only exchange and bounded outbox patterns, with separate protocol tags and exact actual-sender/incarnation checks.

Spindle now has nonblocking diagnostic jobs and explicit uncertainty/cancellation boundaries. That does not make all storage asynchronous: the live Quil code still has pdx_storage_call / pdx_storage_call_bounded, quil_persist_save/load, and Save As paths in servers/quil/src/main.rs. These are the next user-visible bottleneck. Linen already has staged asynchronous snapshots in servers/silk-shell/src/linen_async.rs.

Normal sexnet startup still reports network=0, dns=0, tcp=0, http=0, tls=0 in servers/sexnet/src/main.rs. Experimental network proofs are not an enabled browser stack. The architecture inventory still has 7 open and 23 partial obligations; complete epoch replay, hostile-code containment and SMP are not established.

## Execute next: one five-round document slice

Complete these in order. Keep each round reviewable, record source/image hashes, and freeze evidence before moving on.

| Round | Implementation | Finish only when |
|---|---|---|
| 14 | Inventory every reachable Quil storage wait; introduce a typed, correlated, bounded main-loop document loader over the existing BOOT routes. Stage the document privately and swap after validation. | Real Linen selection opens the exact requested object; wrong sender/sequence, stale selection, partial data and timeout cannot replace the editor buffer. Type and Close continue while load is pending. |
| 15 | Convert Quil save and Save As into asynchronous revision-bound jobs. Preserve dirty state across edits during save; keep the last complete saved version until the new version commits. | Edit during save, then verify the completed receipt names the saved revision and leaves newer edits dirty. Two different objects retain distinct bytes after close/reopen. Never mark Saved on admission. |
| 16 | Complete durable document transaction semantics using explicit service-owned selected-object bindings and BOOT-sanctioned object access. Add flush/readback and old-or-new recovery tests. | Two documents survive reboot byte-for-byte; interrupted save yields the previous complete version or the fully committed replacement. Failure/cancellation never promises rollback when a commit may already exist. A foreign or recycled object reference is rejected before effects. |
| 17 | Finish document workflow UX: New/Open/Save/Save As, dirty/saving/error/uncertain states, keyboard focus and readable editor/caret at small tiles. Use shared spacing/type/focus tokens. | A keyboard and pointer walkthrough produces useful documents without typing diagnostic commands. Screenshots verify actual text/caret/labels at 1280x800 and 1024x768, including resize and minimize/restore. |
| 18 | Integrate both apps and run the alpha candidate session. Preserve source/image/disk bindings, then commit and prepare the next bounded lifecycle slice. | A 30-minute automated session edits two objects, saves, refreshes Linen, switches focus, exercises pending-I/O Close, relaunches within the explicit BOOT reserve, reboots and compares exact bytes with no lost input or leaked private backing. |

Primary files: servers/quil/src/main.rs; a new small Quil exchange module; servers/sexfiles/src/trampoline.rs and a dedicated document protocol/service module; crates/sex-pdx/src; servers/silk-shell/src/linen_async.rs; shared frame/content layout; focused production-state host tests and native QMP gates. Do not turn the monolithic editor into a broad cleanup project before the working vertical slice is complete.

The storage probe service remains diagnostic. Its pattern-fill commands are not a general document save API, and a numeric disk slot is not a new capability. Design document authority explicitly rather than silently extending the diagnostic scope.

## Alpha blockers after the document slice

1. **Repeatable app lifecycles.** The current BOOT reserve permits one successor per Quil/Spindle app and then reports exhaustion. Increase capacity only through explicit BOOT provisioning of fresh identities, routes, catalogs and backing, with aggregate table/budget tests. Show remaining reserve clearly. Long-lived daily use requires a separately reviewed BOOT-sanctioned incarnation-family/replenishment design; never solve exhaustion by reviving a dead ID or minting runtime capabilities. A larger finite pool is an alpha limit, not unlimited relaunch.
2. **Reliability and recovery.** Run repeated boot/session/reboot gates, storage fault injection, mixed input/service load and resource accounting. Close must retire service state and invalidate translations before Sext zero/reuse. Keep a recovery boot path and reproducible image/disk copies. Resolve failures before adding more apps.
3. **A coherent desktop.** Consolidate typography, spacing, focus contrast, title-bar hit targets and loading/error states across the working apps. Validate visible pixels and actual keyboard/pointer actions. Avoid adding decorative surfaces that consume the fixed surface budget without improving the document workflow.
4. **Real networking, then online use.** First prove controlled host/guest TCP request/response bytes with retries, checksums, timeout, disconnect and reconnect; then DNS and HTTP integration, then TLS and the intended browser/document viewer. Keep mocks off in acceptance runs. No claim of Internet usability before these pass in the normal image.
5. **Named hardware.** Choose one x86_64 test platform after VM reliability, document boot/display/NVMe/input/NIC support and DMA boundaries, then run the same workflow on it. Physical SMP and ARM protection remain separate engineering gates. The Apple Silicon development host running x86 QEMU is not proof of a native Apple desktop port.

## Release checklist

The QEMU alpha may be tagged only with a named image and disk format, repeatable normal boot, usable document workflow, explicit failure/uncertainty states, complete native glyph/input evidence, a passing sustained session, and a clear list of reserve and security limits. Run the sealed master gate, every architectural host gate and Python checker tests; count actual tests, not empty Cargo invocations.

The daily-driver label additionally requires a sustainable app lifecycle, validated storage recovery, the user's required network workflow, acceptable measured responsiveness on the named target, and honest protection/replay claims. Record latency measurements as observed distributions; do not infer hard real-time guarantees from one successful run. Fix outstanding authority bypasses before allowing untrusted code.

## Start the next session with this request

Continue from the completed Spindle storage report. Execute rounds 14–18 above, starting with nonblocking Quil document loading and saving, then durable two-object recovery, readable editing UX and a sustained native alpha session. Preserve GLOBAL_VAS, BOOT-only capability genesis, sex-pdx authority and terminal incarnation identities; SexOS is not Unix. Use the current source as truth, keep model delegation low, verify each round, commit the result, and update this plan with the measured blockers. Do not claim a daily driver while its required release gates remain open.
