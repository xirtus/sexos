# Next Spindle milestones: held recovery roster before reserve growth

Keep SexOS a single-address-space microkernel. Catalog metadata selects expected
associations; BOOT creates authority, typed sex-pdx queries held grants, and PKU
enforces it. No runtime capability mint, per-process VAS or Unix/POSIX interface.

First retain **two actual Spindle incarnations** while removing the remaining
pair-specific recovery mechanism. Use an owned bounded history recovery grant
with `(target, main_task)` participants sized by the Spindle catalog, plus
holder/logical identity and the existing disk extent. BOOT must populate actual
registered main-task identities before sealing. Catalog membership cannot
substitute for the held grant. Keep the current `SEXHST01` format, two banks,
18-sector reservation, 4096-byte snapshots and generation/flush rules.

The proposed syscall67 metadata query is:

| Kind | Returned metadata after OK |
|---|---|
| 0 | logical, holder, participant count, version |
| 1 | base, sectors, zero, zero |
| 2+i | target, actual main task, i, zero |

The decoder must require exact bounded count/version/order, complete enumeration,
nonzero full-width identities, unique subjects and tasks, expected family,
reserved zeros and valid extent. Invalid/missing authority must redact metadata
and cause zero disk operations. Kernel query remains tied to the actual held
capability slot. Test non-equal task/domain identities, high bits, duplicate and
unknown writers, truncated queries, wrong holder/extent and missing grants.

A read-only review of the sealed expanded image finds a 128-byte
`CapabilityData`, 192-byte separately allocated capability with alignment64,
and a 12KiB table holding 1024 pointers. The proposed grant is 40/48/88 bytes for
2/3/8 participants; it should fit the existing capability allocation through
eight. These hypothetical layouts were not compiled: verify with actual size
assertions during implementation. A kernel static-reference roster saves no
current enclosing allocation and adds lifetime/publication plumbing; prefer
owned data at this bound.

Migrate catalog-sized history and storage-probe sessions, collision-free
owner-private names, reply destinations and held-lifecycle checks. Record the
held roster's exact main task in persisted journal metadata rather than assuming
`task == domain`. Preserve isolation, pending Full receipts, retry identity,
retirement and all existing torn-write/crash-point tests. Generalize Linen's
read-only observer grants and the native cold-recovery checker too.

The current transport drops `caller_task` across fresh/deferred message paths
in trampoline, PDX replay, defer queue and nested disk waits. Persisting a
**BOOT-attested main-task identity** does not authenticate each request's
executor. Keep that scope explicit. Before claiming per-request task authority,
carry the kernel-supplied caller task through every such path and reject wrong
tasks before session/receipt mutation; never reconstruct it from caller domain.

Then add **one third Spindle** through BOOT, not an unbounded runtime relaunch.
Qualify all three through actual save, Close, relaunch and latest-writer cold
recovery into a fresh original. Check exact five-role cleanup, private-page
retirement, owner-private handle isolation, stale/wrong-task requests, queued
Full receipts and exhausted-reserve launcher pixels at both desktop sizes.
Grow further only after this complete chain passes.

Implementation groups: grant/SDK/kernel BOOT and authority tests; service/journal/
probe and storage tests; observer/native recovery checks; separate caller-task
transport propagation if its stronger authentication is claimed. Preserve the
obligation inventory: **2 Open / 28 Partial / System Proven false**. Sustained
virtual-machine sessions do not establish physical power-loss, SMP, hostile
containment, global epoch/replay or unrestricted daily-driver readiness.


Round140 implements the first phase; see the
[qualified roster report](SPINDLE_RECOVERY_ROSTER_2026_10_31.md). The actual
reserve remains two. The current app startup task/domain equality check and
per-request caller-task propagation remain explicit follow-up work; native
third-incarnation evidence has not been produced.


## Progress after rounds140–144 and next work

Round140 delivered the held generic recovery roster. Round142 removed remaining
Quil/Spindle startup/task inference through the status/Linen/Shell path. Round143
qualifies actual12→17→24, probes/network release and cold24→fresh12 recovery.
Round144 accepts the distinct older two-Spindle image over paired1800s sessions.
Per-request deferred caller-task authentication remains separate work.

1. Grow the actual reserve to eight: proposed targets25–29, five-surface groups
   starting318, and four-slot groups134–153. Expect17 catalog subjects,29/32
   main self-waits,51 desktop tracked surfaces and an88-byte recovery grant.
   Verify every actual inserted grant/layout before sealing. Shell's retained
   lifecycle table reaches its32-record limit; derive/assert the complete budget
   before spending it, with explicit last-slot and overflow controls.
2. Extend the native reserve probe for every successor. Its current loop ends
   history with `net-probe`, so later Up cannot expect an echo. Save and verify a
   final distinctive echo before each Close. Cover all eight, exact last-writer
   cold recovery, controlled peer release, no stale handles and exhaustion at
   both sizes. Add reserves through BOOT data; do not create runtime authority.
3. Complete separate sustained acceptance on each newly selected production
   image. Keep prior images/results immutable and pending-I/O evidence distinct
   from completed-save Close.
4. Add BOOT-reserved Browser successors with their own bounded memory, network,
   TLS/entropy/trust grants and exact retirement fences. First expand the
   retained metadata budget explicitly; do not treat catalog membership or an
   old Browser's capabilities as successor authority.
5. Improve adaptive Spindle typography. At1024-wide side-by-side tiles the
   current quadrant threshold selects tiny1× text despite substantial empty
   space. Choose readable2× glyphs with fewer source-derived columns, retain the
   prompt tail and exact unused-cell erasure, and prove small/large/Zoom layouts.
6. Daily-driver reuse requires a designed bounded incarnation/generation cycle,
   with BOOT-sanctioned transition authority and stale-reference rejection.
   Finite reserves are useful progress, not unrestricted reuse or global-epoch
   proof. Complete caller-task propagation and app-owned durable workflows as
   explicit authority milestones, then repeat whole-desktop acceptance.


Round145 completes the actual eight-reserve phase and the exact32-entry retained
lifecycle budget, with current/future extracted behavior controls. Both sizes
exercise all seven transitions, final echoes, all twelve new-reserve controlled
network releases, resource cleanup and cold29→fresh12 recovery. The reserve
remains finite. Next implementation: readable2× small-tile Spindle text with
source-bound layout/render/damage oracles; then Browser successors using the now
explicit metadata budget and their own BOOT-held authority. Sustained evidence
for the earlier three-reserve image continues independently; do not relabel it.


Round146 completes readable2× tiles, matching source-bound rendering/oracles,
native tiled/Zoom/exactrestore at both sizes and whole-desktop regressions.
Future cycle timing was corrected without relaxing acceptance. Round147 accepts
the distinct older eight-reserve image over paired1800s sessions; it does not
accept the newer font/timing image. Next: follow the Browser authority plan,
starting with held identity/task propagation while keeping only Browser15 actual.
Catalog rows30/31 alone are unsafe until presentation/input and individual
BOOT-held entropy/clock/trust/network/paging edges are ready.
