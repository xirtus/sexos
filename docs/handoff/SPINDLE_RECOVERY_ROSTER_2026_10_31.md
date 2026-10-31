# Round 140: held BOOT Spindle recovery roster

Source `93a50598` replaces the original/successor recovery pair with an owned,
bounded roster of `(domain, registered main task)` participants. The real
Spindle reserve remains **12/17**. This is the first implementation phase of the
[roster plan](SPINDLE_RECOVERY_ROSTER_PLAN_2026_10_31.md), before adding a third.

BOOT obtains each actual main task from its registered domain, creates the
history grant, inserts it into SexFiles slot53 and queries every published field
back through the kernel capability lookup. Any mismatch fails before sealing.
The versioned SDK query enumerates bounded participants and rejects wrong holder,
count/version/order, reserved fields, high bits, duplicate subjects/tasks and
truncated results. Source catalog membership only selects expected associations;
the held query supplies authority. Linen observer grants follow the same Spindle
catalog while retaining read-only lifecycle rights and no retirement ACK.

History and storage-probe sessions, reply destinations and private names now
follow the catalog. Names preserve existing `spindle-history-12/17` objects and
remain caller-owned. Both pumps retain exact Full receipts, retry/abort and
retirement behavior, with fair round-robin polling capped at two sends.
Temporary three-subject service fixtures reject old reply destinations, cursor
starvation and unbounded-send regressions; they create no real BOOT authority.

The journal records the held roster's main-task identity rather than synthesizing
it from the caller domain. Recovery requires the exact persisted writer/task
pair before copying and rebinding data under a fresh owner's handle. The existing
`SEXHST01` format, two banks, 18-sector extent, snapshot bounds and flush/readback
rules remain unchanged. Host tests cover three writers with unequal domain/task
IDs, latest-writer recovery, authorized older-bank fallback, wrong-task rejection,
missing authority with zero disk operations and existing torn-write/crash points.

This is **BOOT-attested main-task identity**, not authenticated request-task
identity. Fresh/deferred message paths still omit caller-task fields. Spindle's
app startup also still compares the catalog task with the incarnation domain;
unequal task IDs are proven in roster/journal fixtures, not end to end in apps.
Neither stronger property is claimed here. The next app-identity step must
address that distinction before broader task allocation or reserve growth.

Native checkers join the new grant header, actual BOOT catalog tasks and durable
writer. Parsing a checksum-valid disk snapshot alone is explicitly separate from
admitting its writer through that verified roster. Cold and viewport probes stop
the sole disk writer before terminal disk inspection. The optional last-writer
cold probe replays its bound parent and verifies exact journal bytes, fresh own
handle and visible Up recall; it has no standalone read-only artifact replay and
is recorded as an auxiliary observation.

Qualification passes sealed GREEN build/probe, 43 registered host checks /
2,698 Rust executions and 989 Python tests. Both complete eight-Quil/Spindle
relaunch workflows and connected live/cold sessions pass at both desktop sizes,
with bound artifact replay: six normal workflow boots and 84 registry-verified
captures. Both relaunch parents also pass fresh separate replay commands. Including the sealed probe and auxiliary last-writer cold boot gives
eight qualified boots. Every one verifies the inserted history grant, all eleven
catalog subjects and pre-seal ordering. The auxiliary cold boot recovers writer17/
task17 generation 5, 100 bytes, into fresh Spindle12 and visibly recalls
`echo alpha17`; its screenshot is separate from the registry capture count.

An original auxiliary attempt supplied an obsolete `echo spindle17` expectation.
Its recovery was exact, but the driver awaited the wrong recall length. That
owned guest was stopped and its failed result retained. The successful retry
uses the actual last command decoded from the bound parent, on identical frozen
source/image. A preflight journal test also corrected its oracle to accept an
authorized older bank and separately reject two unauthorized banks.

Native layout remains grant 40 bytes, capability data 128, capability record 192;
no capability-record growth is observed. Actual host layouts for two/three/eight
roster entries are 40/48/88 bytes. Six ISO-extracted app/service ELFs, including
SexFiles, scan 254,656 decoded instructions with no inspected vector/FP matches.
That is a static scan, not runtime register/cache erasure.

All 576 production and 589 runner inputs, 1,165 distinct files, match committed
Git blobs. [Machine evidence](assets/sexos-spindle-recovery-roster-2026-10-31-evidence.json)
binds the [raw archive](assets/sexos-spindle-recovery-roster-2026-10-31-raw.tar.gz),
including frozen source/image, host controls, immutable normal workflow frames/
prefixes, compressed terminal disks, fresh replays, auxiliary observation and
original diagnostics. Every member is independently size/hash verified.

ISO: `669ce912f1f43f93f543ba77484c0e825a0ee35012b60b8b3d1db015bbf6dcd6`.
Kernel: `b01a0464c6b0d1d3d3afa90684ab0954b2889c1dadb45613c11ab17ba9d41a26`.

GLOBAL_VAS remains addressing, BOOT remains capability genesis, typed sex-pdx
remains authority and PKU remains enforcement. No per-process VAS, runtime
capability mint or Unix/POSIX mechanism is introduced. The earlier appearance
image's sustained jobs remain separate evidence. Next resolve app startup's
main-task/domain coupling, then add one third Spindle and qualify complete
warm/cold history, cleanup, backpressure and exhaustion before increasing further.
No third-incarnation native, new-image sustained, physical/power-loss, SMP,
hostile-containment, global epoch/replay or unrestricted daily-driver result is
claimed. Inventory remains **2 Open / 28 Partial / System Proven false**.
