# Round 153: isolate unsupported source receipts

Unsupported held Net clients can no longer consume the six retained owner fences
of the authorized entropy, certificate-time and trust machines. After typed
request decoding and actual stamped-task authentication, non-Browser requests
enter a separate error-only outbox before source-machine or provider effects.
Catalog family membership partitions receipt budgets; typed source INFO still
establishes actual consumer/profile authority.

| State | Bound |
| --- | ---: |
| Independent denial protocol partitions | 3 |
| Owner fences per partition | 17 |
| Total denial fences | 51 |
| Static error-metadata backing | 3,272 bytes |

Each owner/kind retains its exact typed DENIED or STALE word and correlation
floor. Full retains that destination and word; duplicates coalesce, a different
pending correlation backpressures, stale requests preserve the newer floor, and
capacity exhaustion never evicts another fence. Correlation-only coalescing is
valid for this denial lane: every unsupported operation with the same decoded
correlation receives the same denial, confirming no ticket or operation.

The new poll path distinguishes live, terminal and uncertain held lifecycle
observations. RETRY, missing/malformed records or mismatched tasks retain and
rotate the pending error/floor. Only exact held terminal target/task with the Net
cleanup role retires it. Cleanup also retires the exact owner before ACK. Review
caught the initial RETRY-as-retirement draft and it was fixed before committing.
Pending denials participate in the nonblocking service loop.

The existing three source machines, source grants, entropy tickets and single
clock high-water/rollback state remain unchanged. The older authorized-source
polling RETRY gap is explicitly deferred to Round 154; this phase fixes the new
lane and its budget isolation, rather than claiming broader liveness coverage.
No new source slots or Browser reservations are installed.

## Verification

Source commit: `7e3a00c3f277d54d3de6192849f1c3bb6bf18637`.
All **43 frozen checks** pass: **2,878 logged Rust executions plus 18 nested cases**
(**2,896 total**), and **1,051 Python tests**. The denial fixture runs 43 cases and
actual compiled dispatch fixture 41, including embedded SDK/machine tests; counts
are executions, not distinct new tests. The new behavior contributes 9 denial
cases and 7 dispatch controls.

Eight explicit Spindle identities with independent host task IDs flood all three
source types; afterward Browser 15/task 150 still obtains 32 seed bytes, positive
RELEASE, time and trust. Provider counts and authorized machine state remain
unchanged for unsupported requests. Tests cover typed errors, Full, duplicates,
capacity, fairness, stale floors, exact retirement and tri-state lifecycle
observations. Four compiled mutants reject cache-floor lowering, owner eviction,
denial bypass and task==PD inference; four dispatch wiring controls are rejected.
Clock poison/high-water and ordinary entropy CANCEL/RELEASE remain intact.

The saturation and negative-lifecycle tests use finite host callbacks around
actual code. Native evidence validates ordinary supported workflows, without an
injected unsupported guest flood or a native unequal-task allocator. Eight boots
and **112 registry-verified captures** pass both desktop sizes, full 8 app chains,
independent rehearsal replays, connected TLS image/document workflows, cleanup
and parent-bound auxiliary cold recovery. Writer 29/task 29 generation 55, 761 bytes
recovers into fresh Spindle 12; Up recalls `echo alpha29`. There is no standalone
cold-artifact replay claim.

All 579 production and 611 runner entries, 1,190 distinct files, match committed
Git blobs, frozen copies and source binding. Nine exact ISO-extracted ELFs contain
326,655 decoded instructions with zero inspected vector/FP matches. This static
scan does not prove runtime register/cache erasure.

ISO SHA256:
`c2bf50a704f16fb0c522f5f9c7e8e7a3b4730a71db043fbe8fa6d85e128718c8`.
Kernel SHA256:
`51615c566debd58ab5e96afeb6ec028618aa60db038b956b5821a52456962851`.
Build ABI source hash remains 575a6a0e. The [evidence](assets/sexos-source-receipt-isolation-2026-11-01-evidence.json)
and [raw archive](assets/sexos-source-receipt-isolation-2026-11-01-raw.tar.gz)
retain qualified artifacts and controls. All 1,552 archive members were reopened
and checked by size and SHA256. Archive 14,582,625 bytes; SHA256:
`85ceb4831143d39a62e90ac600f02c1166e35e0fc8e8b93fc7126f7fb16340bb`.

Next 154 carries tri-state held liveness into authorized-source polling and checks
entropy's original lease even while source work waits. Then the
[per-consumer BOOT source plan](BROWSER_SOURCE_GRANTS_PLAN_2026_11_01.md) generalizes
held slot selection, followed by separate bitmap stores and actual successors.
GLOBAL_VAS remains addressing only, BOOT alone creates capabilities, typed
sex-pdx authorizes and PKU enforces. No process VAS or runtime capability minting
is added. Inventory remains **2 Open, 28 Partial, System Proven false**; new-image
1800-second acceptance and broader protocol gaps remain outside this round.
