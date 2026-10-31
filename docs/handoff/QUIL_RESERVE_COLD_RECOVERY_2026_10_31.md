# Round 137: cold recovery after the final Quil incarnation

Source `9e0edd28` adds a separate cold-recovery gate for the exact terminal disk
of a completed eight-Quil session. A new BOOT opens the last Quil23 save in fresh
Quil9. This closes the post-final-incarnation cold-boot gap recorded in
[round 136](QUIL_EIGHT_RESERVE_2026_10_31.md).

The parent must pass full immutable alpha replay, including every exact
activation edge, warm document load, edit/save, cleanup and final launcher
exhaustion. The new gate then joins its complete source catalog, final save,
compressed terminal disk SHA-256 and independently decoded journal payload.
Cold BOOT receives those exact disk bytes. Actual Linen selection and Open must
produce one ordered native recovery/load chain with identical object identity,
journal generation, bank, byte count and content hash. Full viewport and clock
pixels are captured only after that load. The cold final journal must retain the
same payload and metadata. A fresh independent replay validates these joins
without starting a guest or reading a live device.

The existing pending-I/O cold engine now accepts an explicit source-bound
recovery policy. Its transport, deadline, sole-writer shutdown, immutable frame,
receipt and journal checks remain shared. The default retains the previous
pending-I/O document outcomes and frame contract. Native load checks also reject
any nonprimary Quil startup before the cold Open completes, including the newer
subjects. Tests reject skipped/forged reserve steps, wrong terminal targets,
changed payloads/hashes/seeds, foreign frames and nonprimary cold receipts.

This is a **new checker cohort using the same qualified production image**.
All 576 production inputs, ISO and kernel match round 136. Only four Python
checker/test files differ; all 576 production and 585 runner inputs match their
committed Git blobs. The new cohort runs fresh parent sessions so no earlier
session is relabelled with a different binding. All 975 Python tests pass.
Round 136 supplies the unchanged production/host/static-scan evidence: 43 host
checks, 2,684 passing Rust executions and five exact ELF scans. Those checks are
identified as prior evidence, not presented as newly rerun checks.

Native qualification completes eight new boots and 80 verified captures across
1024×768/e1000 and 1280×800/e1000e. Each profile runs the full eight-incarnation
parent, exact final-save cold recovery, pending-device Close and its default
cold recovery. All eight workflows pass fresh independent replay. Both final
Quil23 saves recover into Quil9 as 20 bytes, generation12, bank0, identity327681,
with hash `0x1066708195bedecd`. All eleven BOOT catalog tuples precede sealing in
every new boot.

[Machine evidence](assets/sexos-quil-reserve-cold-2026-10-31-evidence.json)
records both exact cold ancestry chains, source-cohort comparison and supporting
checks. The [raw archive](assets/sexos-quil-reserve-cold-2026-10-31-raw.tar.gz)
retains the frozen checker/image, fresh parent and cold sessions, compressed
terminal disks, immutable frames/prefixes and independent replays. Every archive
member is independently size/hash verified.

The production image remains ISO
`33d25951837bbf7fdb935d8c36185c9b9cea5f7d11169a66f51c3cd3a65d3bb0` and kernel
`3624ee301ef8c3f7fec7cbaa58fd579f0d8196ae8094d806518aec2d9c756e71`.
No kernel authority or app reservation changes here. GLOBAL_VAS still supplies
addressing, BOOT alone creates capabilities, typed sex-pdx supplies authority,
and PKU enforces it. No Unix/POSIX mechanism or runtime capability mint is added.

The expanded image's separate 1800-second alpha jobs remain live when this
qualification is assembled; they are excluded from this acceptance and archive.
Next verify their terminal results and independent replays, migrate remaining
native captions/focus indicators to catalog metadata, then address Spindle's
journal/recovery ownership before expanding its reserve. Power-loss/general
crash consistency, physical hardware, SMP, hostile containment, global
replay/epoch and unrestricted daily-driver readiness remain outside this result.
Inventory remains **2 Open / 28 Partial / System Proven false**; the goal is active.


The Spindle migration should first keep two actual incarnations while replacing
fixed caller12/17 history/probe sessions and the recovery grant's identity pair
with a bounded, BOOT-populated roster of actual subject/task identities. The
held recovery capability, disk extent and participant tuple must be checked
before disk access; catalog membership alone cannot admit it. Keep the existing
journal format, two banks, reserved extent and snapshot bound. Preserve exact
caller-task identity through deferred messages before claiming request-level
authentication. Tests need unequal task/domain IDs, missing grants with zero disk
operations, unknown/wrong-task writers, retirement isolation and existing torn
writes. Only then add a third Spindle and qualify warm/cold history, full cleanup,
backpressure and exhaustion before growing the reserve further.
