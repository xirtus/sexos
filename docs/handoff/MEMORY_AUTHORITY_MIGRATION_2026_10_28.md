# Bounded memory authority integration

Date: 2026-10-28. Status: runtime candidate integrated after scheduler acceptance;
34 host tests pass and independent source review passes. Fresh sealed image and
runtime evidence are owned by the parent integration report; host results below
do not themselves establish successful boot or hardware execution.

## Implemented boundary

Runtime ambient frame allocation, raw physical mapping and MemLend minting are
unavailable. Syscalls 30/31/50/51 return `u64::MAX`; historical shared-buffer syscall
40 returns its failure sentinel zero. Syscall 43 retains its calling convention
but returns an existing boot PCI BAR0 view after capability/type/bounds validation.
It performs no PCI configuration read, frame/VA allocation or page-table mutation.
The obsolete `ipc::buffer` module export is removed; its allocator has no callers.

New syscalls 54/55 resolve immutable, already-mapped BOOT_DAG capabilities.
`sex-pdx` pure helpers decide authority from trusted kernel capability payloads.
PKU state and caller-supplied physical addresses/descriptors grant no authority.
The kernel query path does not choose budgets, allocate backing frames, create
mappings, change pkeys, or install capabilities. No runtime domain-ID whitelist
substitutes for capability possession.

Bootstrap reserve/zero/map is an explicit exception against the canonical sext
memory model. It uses existing kernel loader/mapping machinery before user domains
run. Sext remains inactive: its source has duplicate entry/panic handlers and stale
APIs, and it is absent from the sealed build. This integration does not implement
a ring-3 pager or a runtime physical frame allocator. Drivers only reserve byte
subregions inside frame authority already assigned at boot.

## Exact ABI

`CapabilityData::Region(RegionGrant)` contains region ID, existing VA, length,
rights, optional DMA base and optional boot-bound peer ID. A copied user descriptor
is descriptive data and cannot become a kernel capability. PCI capabilities carry
an optional embedded BAR0 region; both new queries accept that immutable view.

Rights are `QUERY_VIEW=1`, `READ=2`, `WRITE=4`, `DMA_ADDRESS=8`. Physical addresses
are returned only when DMA_ADDRESS is both requested and granted. Unknown rights,
full-register high-bit aliases, zero lengths, overflow, out-of-range spans,
malformed grants, absent capabilities and wrong capability kinds fail closed.

| Operation | Inputs | Outputs |
| --- | --- | --- |
| 54 region view | RDI slot, RSI offset, RDX length, R10 rights | RAX status, RSI VA, RDX length, R10 DMA PA or zero, R8 region ID |
| 55 region metadata | RDI slot | RAX status, RSI length, RDX rights, R10 peer ID or zero, R8 region ID |
| 43 legacy BAR query | RDI PCI slot, RSI exact BAR index zero, RDX size (zero means 4096) | RAX existing VA or MAX |

54/55 success status is zero; failures return MAX and clear all four output
registers. Slots are decoded at full width before lookup. The production
`syscalls::region_query` helper is directly extracted and tested by the host gate.
54/55 metadata has no physical/virtual address output. 43 rejects oversized sizes
rather than silently mapping/clamping an unowned physical extent.

Public wrappers are `memory_region_info(slot)`,
`memory_region_view(slot, offset, length, rights)` and
`memory_region_full_view(slot, rights)`, returning Result. Callers explicitly
include QUERY_VIEW. `MemoryRegionInfo` reports length/rights/peer/region ID.
`RegionView` reports existing VA, length, optional DMA PA and region ID.

## Fixed boot graph

`kernel/src/boot_memory.rs` contains the explicit initial resource plan. Actual
spawned domain handles and their pkeys are used; runtime queries infer no rights
from numeric domain identity. With all current domains present there are seven
ordinary region grants:

| Region ID | Slot | Holder | Exposed bytes | Rights | Peer |
| --- | --- | --- | --- | --- | --- |
| 1 | 32 | sexusb | 262144 | 15 | none |
| 2 | 32 | sexnet | 262144 | 15 | none |
| 3 | 32 | sexdrive | 524288 | 15 | none |
| 4 | 17 | sexfiles | 4096 | 7 | sexdrive |
| 4 | 17 | sexdrive | 4096 | 7 | sexfiles |
| 5 | 33 | sexdrive | 3145728 | 7 | sexdisplay |
| 5 | 33 | sexdisplay | 3145728 | 3 | sexdrive |

The frame backing uses a 4 MiB buddy block but exposes only 3 MiB. Every entire
RAM reservation, including rounding, is zeroed before mapping/publication. Each
holder gets its own VA alias under its actual pkey. All region mappings are NX;
DMA/block/producer views are writable and the display frame alias is PTE read-only.
No new user pkey-zero alias is added. The former identity-derived shared-pkey14
openings are removed from `PkruValue::for_domain`.

PCI BAR0 extent is probed only during RegistryBuild with I/O/memory decoding and
bus mastering temporarily disabled. Both 64-bit BAR words and the original PCI
command are restored; writes keep status upper bits zero to avoid W1C corruption.
Unsupported kinds, zero addresses, malformed masks, subpage extents, unaligned
bases and out-of-range addresses are rejected. Boot maps at most the actual BAR
extent or a 64 KiB prefix, under the device recipient pkey with UC flags. GPU PCI
caps have no such query view. Region IDs for PCI use `0x10000 + pd*1024 + slot`.
USB/NIC/NVMe query actual bounds rather than assume a 64 KiB aperture.

Any backing allocation, exclusion or mapping failure stops boot. Paired backing
and both aliases are constructed before granting those views. New grants reject
occupied slots. After every ordinary/device/memory grant, all capability tables
are sealed before the first task admission. Sealed table insertions fail closed
by panic; none of the active syscall handlers attempts a mutation after seal.
Dormant legacy syscall source modules remain compiled but are not dispatch routes.

## Bootstrap ownership correction

The existing forward BootInfoFrameAllocator did not exclude data blocks obtained
from the buddy allocator. Reserving only buddy metadata could not stop a later
page-table allocation from taking an interior DMA/data page. Each complete boot
backing interval is now registered with the forward allocator before zeroing.
Registration rejects overlap with the entire consumed usable-memory prefix and
with previous reservations. Page-table allocation skips every reserved page.
Older HAL network diagnostic DMA buffers use this same backing mechanism too.
The fixed interval table holds 32 reservations and fails on exhaustion.

Independent review found another existing bootstrap issue: when the first
remaining region was too short for allocator metadata, metadata was carved from
a later region but the frame cursor skipped only the metadata page count from its
old position. Example: one remaining page in A, two metadata pages in B; the old
skip consumed A plus one B page, then exposed the second metadata page. Free-list
heads could also be published before the metadata array existed and was zeroed.

The production `BootstrapPlanner` now computes the exact physical metadata
interval and all remaining free intervals. That interval is excluded before
metadata initialization; free lists are populated only after initialization.
Tests exercise the actual planner for fragmented regions and consumed prefixes
across holes, plus interval exclusion. No collision is hidden by retrying until a
seemingly successful allocation. The buddy allocator remains a prototype (including
its head-only metadata/coalescing behavior and vacuous verify_invariants hook);
this work is not a general allocator verification or redesign. New bootstrap
reservations reject detected ownership collisions before publication.

## Migrated callers and DMA custody

USB and NIC initialize one local `DmaRegionPool` from slot32, then reserve bounded
permanent subregions with paired PA/VA addresses. No raw allocation/map fallback
remains in those active servers. Sexfiles VFS, backend proofs and runtime proof
resolve static slot17 instead of minting MemLend caps. Negative proofs use absent
slot31, wrong-kind SLOT_BLOCK and denied DMA disclosure on the shared buffer.

Sexdrive uses a 512 KiB local arena. Its two fixed read/write bounce buffers are
reused for normal I/O and write/readback proofs. Startup/proof reservations remain
bounded; the driver lane counted 23 startup pages (94208 bytes). Shared block
requests check static slot17's peer against authenticated `msg.caller_pd`, then
resolve bounded rights. A payload caller ID cannot substitute for that identity.
The frame producer resolves static slot33. Silk-shell still stores its frame
handle opaquely; this does not prove a working UiIR/shared-frame compositor.

`DmaLease` separates Available/Prepared/InFlight/Quarantined.
`DmaTransfer` marks submission immediately before the device-visible doorbell and
requires matching CQ CID and phase for completion. Its exclusive mutable lease
borrow lasts through the function: successful completion is marked before CQ
acknowledgement, but backing cannot be prepared again before the same call returns
after acknowledgement/copy. Status errors set ready=false. An unfinished submitted
transfer quarantines backing on drop and disables further I/O; timeout/fallback
cannot replace or recycle that buffer. There is no invented controller reset or
unquarantine method. Pre-submit cancellation alone can release Prepared backing.

## Evidence and limits

`bash scripts/memory_region_authority_gate.sh`: **34 tests pass**:
12 production region tests, eight production DMA pool/lease tests, three PCI BAR
decoder tests, eight boot reservation/planner tests and three extracted production
region-query tests. The gate checks no allocation/map/grant calls remain in the
active syscall file and reservation precedes backing zeroing. Tests include
100000 completed lease reuses without pool growth, timeout/late-completion denial,
malformed full-width selectors with zero lookups, and zeroed failure outputs.

Existing capability-slot gate: ten tests pass. Existing syscall user-pointer
hardening gate: all 15 checks pass. Diff whitespace check passes. Independent
read-only review of bootstrap metadata placement, interval exclusion, capability
sealing, frame permissions and query handling passes. Driver lane tests and live
sealed-image results are reported by their own handoffs and the parent report.

Boot includes an actual capability-table negative selftest through the same
`region_query` function used by54/55. With the normal graph it expects 15 denials
and 1024 identical ordinary views without physical disclosure. The runtime gate
must observe this marker, all seven grants, sealing before task.running, and the
migrated device/storage paths; source presence alone is not execution evidence.

Syscall52 reads static/atomic network diagnostic results only. Large older HAL
network setup remains explicitly boot-only (`enumerate_bus` requires RegistryBuild),
and its resources are reserved above. Dormant old crate slot-zero memory APIs are
not working migrated applications. Nothing here establishes deterministic device
input epochs, ring-3 demand paging, arbitrary native-code isolation, IOMMU/DMA
confinement, WRPKRU containment or privileged SyncCall confinement. PKU-absent
operation retains the same capability-query decisions but has softer hardware
enforcement. These limitations must not be described as closed by host tests.
