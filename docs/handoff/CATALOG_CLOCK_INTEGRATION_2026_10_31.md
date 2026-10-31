# Catalog, clock and desktop integration

Source merge `63b2f118117f49d08202b9dafe49362221e1a9e2` combines Round 133
(`8faeafbb`) with the complete clock, immutable-capture, sustained-workflow and
pending-device stack through PR #10 (`1c9c64ec`). It preserves both histories.
The workflow merge retains stall-register diagnostics, stop-before-final-disk
capture, find/replace, immutable capture/replay and pending-I/O qualification.

BOOT now audits the descriptive incarnation catalog against actual kernel
objects before sealing or admitting tasks. It checks all six app subjects,
main-task identity and execution-domain bindings, surface/lineage/dormancy,
Shell and self lifecycle grants with exact rights, close/view routes and every
successor activation binding. This audit reads existing objects and grants.
Display frame/child policy remains a separate concern; catalog lookup grants
no authority.

The imported reply ledger silently saturated at 65,535 obligations and could
accept calls from callers beyond its 32-entry tracking capacity. The integrated
ledger indexes the existing 1,024-domain registry, keeps checked 64-bit counters,
and refuses before publication if a reservation cannot advance. Failed enqueue
withdraws exactly one reservation; full reply queues retain the right; successful
reply publication consumes it. Tests consume every one of 100,000 accepted
invocations and cover the exact counter limit, invalid callers and retirement.

The original single-address-space model is preserved: GLOBAL_VAS names memory,
BOOT creates authority, typed held grants authorize operations, and PKU enforces
access. There is no runtime capability mint, process VAS, POSIX layer or R9 IPC.
`ARCHITECTURE.md` remains SHA-256
`4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`.

The exact frozen integrated source passes:

- Sealed `GREEN_MASTER` build and 35-second probe; 576 production and 578
  runner inputs all match committed Git blobs at `63b2f118`.
- 43 registered host/static checks, unchanged inputs, 2,661 passing Rust test
  executions, and 965 Python tests. The new pure BOOT validator has 14 focused
  cases and a compiled wrong-route control that the tests reject.
- 18 workflow boots and 138 immutable verified captures at compact/e1000 and
  wide/e1000e: normal/invalid-year/close clock cases, connected desktop and cold
  recovery, TLS responsiveness, real pending-device Close/cold recovery, and
  find/replace/undo plus app-relaunch rehearsals. Fresh independent replays
  pass for both pending-device pairs and both app rehearsals.
- Six actual catalog audit receipts before memory sealing in every workflow
  boot and the sealed probe: 19 boots total. Document views, NIC command custody
  and bounded self-wait log checks also pass.
- Five exact ISO-extracted app/service ELFs: 225,385 decoded instructions,
  no inspected vector/FP matches. This static scan does not prove runtime
  register/cache erasure or complete instruction-set coverage.

ISO: `7bea8af9c9cd506a498f13e955afaa8f3d60397adfb9c0f0addf578fd98a2fef`.
Kernel: `30cbf36a7a672f2a123cfd0ab19e9b4de484f3d7dda55007d566f4b57b04fea8`.
[Machine evidence](assets/sexos-catalog-clock-integration-2026-10-31-evidence.json)
binds the [raw archive](assets/sexos-catalog-clock-integration-2026-10-31-raw.tar.gz),
including frozen source/image, all native records, frames and disk snapshots,
host logs, fresh replays and original integration diagnostics. Every archive
member is size/hash verified. Earlier failed/pre-enhancement observations remain
diagnostic; none substitutes for the final frozen cohort.

The [earlier optimized image's two completed 30-minute sessions](ALPHA_PAIRED_ACCEPTANCE_2026_10_31.md)
are archived separately: 172 cycles/418 captures. The integrated image has short
rehearsals; this report does not claim 30-minute acceptance for the new image.

The next alpha work is to make the finite BOOT reserve large enough for normal
desktop use. The current catalog still contains three Quil, two Spindle and one
Browser incarnation. First derive remaining lifecycle/capacity/document-client
bounds from that catalog and reject collisions/budget overflow at BOOT. Then
reserve additional images and exact service/route/grant edges, including Browser
successors, with the same predecessor-death/cleanup/leaf-restoration rules.
Prove repeated close/reopen, explicit exhaustion and responsive launcher state
through native sessions before calling reserve expansion complete. Follow with
a sustained session on that expanded integrated image, then hardware/storage,
network availability and desktop polish work.

These are virtual, controlled-profile results. Physical/power-loss/SMP/hostile
containment and global epoch/replay obligations remain. Inventory stays
**2 Open / 28 Partial / System Proven false**; the overall daily-driver goal is
active.
