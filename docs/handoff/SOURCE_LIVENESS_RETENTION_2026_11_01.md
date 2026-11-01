# Round 154: retain source custody through lifecycle retries

Authorized entropy, certificate-time and trust work now distinguishes LIVE,
terminal and uncertain held lifecycle observations. RETRY, unavailable or malformed
records, and mismatched tasks preserve retained source state and correlation
floors. Replies defer without changing destination. Only exact held terminal
proof with the Net role retires source custody.

Entropy samples only after a positive live attestation. Its new lease-maintenance
method checks the existing deadline without source work or renewal. Partial and
ready seeds survive temporary uncertainty before that deadline; expiry or backward
ticks still scrub bytes and redact pending/cached READ payloads. CANCEL/RELEASE,
tickets and existing sequence fences remain intact.

Queries themselves can take time. Maintenance therefore reads fresh ticks after
liveness. Delivery peeks only the pending owner, performs the query, checks the
lease with fresh ticks, then copies the pending word and checks its owner before
sending. The old timing and early-copy implementations fail the new runtime
counterexamples. There is still one certificate-time machine; high-water,
rollback poison and global source ordinals are unchanged.

The older generic TCP/DNS maintenance, packet and fresh-request lookup paths
remain outside this bounded source fix. No new grant, source slot, app identity,
clock fallback or runtime capability creation is introduced.

## Verification

Source commit: `d0b2a163cd2f5a4d77a5999223774311b3ed4382`.
All **43 frozen checks** pass: **2,892 logged Rust executions plus 20 nested cases**
(**2,912 total**) and **1,051 Python tests**. The lease fixture executes 33 cases,
including 5 new lease controls; the actual dispatch fixture executes 50, including
finite positive-LIVE queries advancing ticks across the deadline or backward.
The lease checker rejects two compiled noop/renewal mutants; dispatch rejects
six compiled mutants and nine wiring controls. Existing denial/source controls
remain registered. These are execution counts, including reused SDK tests.

Eight boots and **112 registry-verified captures** pass both desktop sizes,
full 8 app chains, fresh rehearsal replays, connected TLS image/document workflows,
cleanup and parent-bound cold recovery. Writer 29/task 29 generation 55, 761 bytes
recovers into fresh Spindle 12; Up recalls `echo alpha29`. The cold observation
has no standalone artifact-replay claim. Native sessions validate ordinary
supported workflows; forced contention, query-time tick jumps and unequal task
namespaces are covered by compiled host callbacks rather than a native injector.

All 579 production and 612 runner entries, 1,191 distinct files, match committed
Git blobs, frozen copies and their binding. Nine exact ISO-extracted ELFs contain
327,008 decoded instructions with zero inspected vector/FP matches. The static
scan establishes no runtime register/cache erasure proof.

ISO SHA256:
`bd9991bf8b7fea48a15683411472f5251825fda29ed25c04d0068b6c81b8a13d`.
Kernel SHA256:
`51615c566debd58ab5e96afeb6ec028618aa60db038b956b5821a52456962851`.
Build ABI source hash remains 575a6a0e.

The [evidence](assets/sexos-source-liveness-retention-2026-11-01-evidence.json)
and [raw archive](assets/sexos-source-liveness-retention-2026-11-01-raw.tar.gz)
retain qualified inputs, runtime artifacts and controls. All 1,554 archive members
were reopened and checked by size and SHA256. Archive 15,283,454 bytes; SHA256:
`9e5febc5cf69ee4b7531dc512eb6ecd4ec66c3fe4fab8da8a0ae7b8650e80b9c`.

Next 155 implements the [per-consumer source plan](BROWSER_SOURCE_GRANTS_PLAN_2026_11_01.md):
current catalog-derived slots, checked actual BOOT insertion/readback, and immutable
Net associations from held lifecycle/typed INFO queries. It still creates only
Browser 15's current three source grants. Future 70–75 slots are descriptive until
actual successor rows, grants, backing and activation are separately installed.

GLOBAL_VAS remains addressing only, BOOT alone creates capabilities, typed
sex-pdx authorizes and PKU enforces. There are no process address spaces or
runtime capability mints. Inventory remains **2 Open, 28 Partial, System Proven
false**. Broader liveness/protocol gaps, physical/SMP containment and new-image
1800-second acceptance remain outside this qualification.
