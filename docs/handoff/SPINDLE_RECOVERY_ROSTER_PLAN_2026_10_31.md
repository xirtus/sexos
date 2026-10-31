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
