# Round 160: bounded BOOT main inventory at 64

The scheduler's BOOT main-task inventory now uses one explicit
`self_wait::BOOT_MAIN_CAPACITY = 64` constant. Both its storage and seal receipt
use that constant; the receipt also reports the actual `MAX_TICKS`. The former
32-main ceiling would have rejected the next reserve expansion during BOOT.
The current catalog is unchanged: 19 app reservations and 31 registered mains.

Registration still retains exact main-task pointers during RegistryBuild, seals
once before memory-authority sealing, and rejects duplicate, null, occupied,
out-of-range and postseal entries. A runtime wait still requires the exact
current task to match its protection domain's main task. No runtime registration,
lookup-based admission, capability mint or extra app activation was added.

The pointer array adds 256 bytes on this 64-bit target. Timer polling and
membership checks can examine up to 64 slots, including empty ones. Cancellation,
single-claim deadlines and existing wake/stack custody remain unchanged. This is
bounded capacity growth; no scheduling speed or latency improvement is claimed.

The native checker independently requires the reviewed capacity of 64. It rejects
malformed, truncated, duplicate, noncanonical and out-of-range seal records,
including a malformed extra marker after a valid record. It cannot accept an
arbitrary larger capacity just because a log reports one.

A separate stale assumption surfaced during review: sampled timer parks from
Browser successors 30 or 31 were rejected by the old original-task-only checker.
The checker now accepts an explicit, validated list of observed Browser task
identities. This cohort's auditor joins that list to the source-validated typed
source inventory and actual BOOT app records. It does not discover authority
from arbitrary park events or assume task IDs equal domain IDs. The default
original-task profile remains explicit; tests also exercise unequal task 99.
Unequal domain/task allocation has not been demonstrated natively here.

## Validation and evidence

- All 43 frozen host checks pass without source drift: 3,081 logged Rust test
  executions plus 27 nested executions, totaling 3,108. The self-wait helper has
  12 tests covering all 64 exact pointers in slot order, sparse inventories,
  upper slots 32/33/64, rejection of entry 65, cancellation, one wake and terminal
  custody. Compiled capacity-32/65 mutants and six source-wiring mutations fail
  as required. These are atomic-helper/source tests, not 64-task native or SMP
  execution evidence.
- All 1,118 frozen Python tests pass. The checker tests preserve active/inactive
  and retained-reply rules while checking capacity boundaries, explicit task
  selection, foreign tasks, wrong intervals and invalid argument types.
- Three final boots pass: a sealed 35-second boot and allocated-Close Browser
  chains at 1024×768/e1000 and 1280×800/e1000e. Each boot reports 31 mains with
  capacity 64 and has six sampled Net park→expiry→Ring3-return joins. Both native
  runs and fresh read-only replays pass: 20 qualified captures (18 TLS panels and
  two launcher views), six live allocated-future Closes, complete resource ACKs,
  retirement, zeroed pools and finite Browser exhaustion.
- All 1,209 frozen source blobs (582 production and 627 runner; 11,962,343 bytes)
  match pinned Git content and manifests. Nine exact ISO ELFs contain 331,077
  decoded instructions with zero FP/vector scan matches. This static scan does
  not prove runtime register or cache erasure.

Source: `e601e78a91cfe4ffe11b8ec47483bd98922fe7df`.
ISO: `ccec023e4665ce61ccec3d289aa9875f4c86764dbd7b99f9919671ee3b0229f7`.
Kernel: `a2b64dd90d648859c12d97fc3da113330f7016a78a488d67f40541969381fb41`.
ABI remains 66 inputs/hash
`12d218c65235f1600843213a8cdce2cb5f93c0476db09080903171099fe0720f`.
Architecture remains unchanged:
`4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`.

[Raw evidence](assets/sexos-boot-main-capacity-2026-11-01-raw.tar.gz):
9,477,956 bytes; SHA-256
`151c346437eb4c55733ab8e8c6bcb5c3b1bbc6dde87bd11474a453f8ff02d837`.
All 1,359 members were reopened and checked against their size/SHA manifest.
The archive includes frozen source/image/kernel, host logs and controls, explicit
three-boot audit, native captures/actions/PCAP and terminal compressed disks.

The kernel changed. Prior [desktop](BROWSER_TLS_PANELS_2026_11_01.md) and
[teardown](BROWSER_ALLOCATED_CLOSE_2026_11_01.md) evidence keeps its original
bindings. This round claims no new sustained Alpha/connected desktop acceptance,
physical-hardware result or timing guarantee. Architectural inventory remains
**2 Open / 28 Partial / System Proven false**. GLOBAL_VAS, BOOT-only authority
genesis, typed sex-pdx and PKU remain the Sex Microkernel design.

## Next: actual reserve growth

Round 161 targets 16 Quil, nine Spindle and three Browser reservations: 28 apps
and 40 actual BOOT mains. Preserve every existing row and grant, append disjoint
identities/surfaces/holder-local slots, and audit actual grants before sealing.
Measure the kernel heap at the final BOOT boundary and account actual private
pages and document buffers; current free-frame diagnostics are not a terminal
memory watermark. Qualify both full app chains, retirement, fairness and finite
exhaustion before sustained desktop/cold-recovery acceptance.

The draft also identified a real size constraint: 28 app rows would grow Net's
public source-denial cache beyond its existing 4 KiB budget. Compact the redundant
cached DENIED word while retaining exact pending words, latest-sequence floors,
backpressure and fair delivery; retain the budget and test the legacy larger
representation as a failing control. No source provider or authority is created.
