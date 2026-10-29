# Sext capability-backed paging and fault recovery

Date: 2026-10-28. Continuation of scheduler custody and bounded memory authority.
The existing uncommitted work is preserved. `ARCHITECTURE.md` is unchanged.

## Implemented slice

Sext is now a real ring-3 server in the sealed image. It selects frames from an
eight-frame BOOT_DAG pool, zeroes a selected 4096-byte frame through its authorized
alias, and decides whether to commit or reject a fault. Its four-page client quota
is stricter than the mapping capability's eight-page bound: this is userland
paging policy rather than a kernel allocation fallback.

The current boot graph authorizes kaleidoscope's registered main task and one
eight-page demand range. It is a working bounded pager, not general demand paging
for every domain, task, heap, or arbitrary global address. Sext occupies the
previously unused domain/key14 position; kaleidoscope is now domain/key15. Existing
IDs 1–13 retain their meanings. Runtime authorization uses actual capability payloads
and authenticated identities, not an ID whitelist.

The system still has **one GLOBAL_VAS**. No process address spaces, per-process
page tables, Unix VM APIs, runtime capability minting or ambient raw mapping
fallbacks were introduced. Syscalls 30/31/40/50/51 remain denied. Existing region
queries 54/55 remain separate from paging authority.

## Authority and privileged mechanism

BOOT_DAG creates three distinct capability kinds before sealing:

| Holder / slot | Authority |
| --- | --- |
| Sext / 34 FramePool | Eight frames with a writable/NX pager alias; opaque indices, no physical addresses disclosed. |
| Sext / 36 PagerMapping | Bind authorized frames to one target's reserved eight-page range, with bounded R/W rights. |
| Client / 35 DemandRegion | Name its authorized demand range; leaves start absent. |
| Client / 2 Domain edge | Send fault notifications to the actual sext domain. |

Bootstrap reserves and zeroes initial backing, reserves global VA names, creates
the intermediate page-table structure, removes all eight target leaves, and pins
their existing entry addresses. Whole backing reservations remain excluded from
the boot page-table allocator. Capability tables seal before task admission.

Sext owns runtime frame selection, zeroing, quotas and mapping decisions. The
ring-0 operation is a narrow sex-pdx-authorized leaf transition: validate the
invoked capabilities and exact pending transaction, verify the selected frame is
zero, publish that frame into the already-existing absent leaf with the target's
boot pkey and authorized R/W/NX flags, and invalidate the local TLB entry. It does
not choose a frame or invoke a frame, VA or page-table allocator, generic mapper,
memory-zeroing routine, or capability insertion operation. This explicitly
separates the required privileged PTE mechanism from paging authority/policy.

Initial fixed resource reservation remains kernel bootstrap debt. This is not a
claim that all kernel boot memory management has moved into sext. Sext deliberately
retains its authorized RW alias over committed frames; client isolation from a
hostile pager is not supplied by this slice.

## Fault transaction and recovery

Only nonpresent user data reads/writes (x86 error 4/6, nonzero lower canonical
addresses, user CS) enter this recovery protocol. Protection, PKU, reserved-bit,
instruction-fetch, null and kernel faults cannot become demand mappings.
Unsupported non-main tasks fail before enqueue. Task IDs are checked separately
from protection-domain IDs; equality is not assumed.

Before publication the handler saves the entire GPR/IRET context and establishes
fault-specific custody. A global monotonic nonwrapping token binds the exact task,
target, authenticated pager, fault address/page/error and saved frame/RIP. Fault
notifications use opcode 0x103 with address/error/token by value and caller identity
from transport. No pointer to the exception stack crosses domains.

Prepared → Pending publication handles an early concurrent query by returning
RETRY. Queue rejection cancels the unpublished transaction and exits the faulting
task. The pager queries pending metadata, selects and zeroes a frame, then commits.
Pending → Claimed → Committed retires mutable fault metadata before fault-specific
ownership becomes runnable. Existing scheduler custody prevents execution before
the outgoing stack is released. No saved context is rewritten after waking it.

Generic IPC/reply/IRQ wakes cannot complete an unresolved fault. Completion must
use the authenticated transaction and authorized mapping/pool capabilities.
Explicit rejection consumes only that request and exits its saved task. Pending
faults outside the map range can still be queried and rejected by sext policy;
commit never gains out-of-range mapping rights.

Each selected frame and target page has one immutable execution binding. Exact
duplicate commits return success without re-zeroing, remapping or waking again;
conflicting frame/rights tuples fail. Before a PTE is published, failures leave
PTE/ledger unchanged and roll back the claim. After publication, every completion
failure reports RETRY and retains the same frame. A possibly published frame is
never refunded, replaced or zeroed again.

## ABI and active client proof

| Syscall | Inputs | Outputs |
| --- | --- | --- |
| 56 commit | Mapping cap, pool cap, frame index, token, rights in RDI/RSI/RDX/R10/R8 | RAX status |
| 57 query | Mapping cap, token in RDI/RSI | RAX status; RSI page; RDX error; R10 target; R8 rights |
| 58 reject | Mapping cap, token in RDI/RSI | RAX status |
| 59 info | Capability slot in RDI | RAX status; RSI base; RDX bytes; R10 target or 0 for pool; R8 rights |

Statuses: success 0, invalid 1, retry 2, denied 3, exhausted 4. Invalid query/info replies
clear all four data registers. Full-width slots, indices, rights and tokens are
validated before side effects. The contract, ABI input hashes and `.sexspec`
inventory are updated with the implementation.

The real kaleidoscope client probes invalid/high-alias slots and wrong capability
kinds through live syscalls. It then touches four initially absent pages: read
faults on pages 0/2 and write faults on pages 1/3. It verifies zero-filled contents,
successful store/load instruction retry, untouched zero page tails, and retained
values after several faults/context switches. Sext also invokes exact-duplicate,
conflicting-frame, zero-token and unknown-token commit probes in the guest.

The separately declared `paging_quota_proof` client cfg touches a fifth valid
in-range page. Sext rejects it under its four-page policy. The runtime checker
requires the same nonzero saved task to be rejected, no observed subsequent
execution of that task, and successful clock-domain IPC after rejection. Normal
client/kernel flags are restored before final image acceptance.

## Review findings and rejected evidence

Independent Astra reviews found and fixed two protocol defects before acceptance:

- A post-PTE completion error could have looked like a safe unpublished failure.
  All such outcomes now retain custody through RETRY; an actual production commit
  regression fails on the old behavior.
- Rejecting query metadata for an out-of-range pending fault stranded its client
  because sext correctly ignored untrusted notices. Query now permits authenticated
  metadata needed for explicit policy rejection, while commit still denies mapping.

The first boot in `/tmp/sexos-sext-first-20261028/` actually recovered four faults,
but split serial records prevented strict acceptance. Bounded records now format
a whole line before one raw-print syscall, so BSP timer output cannot split fields.

That exposed a preexisting compiler-runtime defect in `silk-client`: its `memcpy`
used `copy_from_slice`, which optimized into a jump back to itself. The first quota
image in `/tmp/sexos-sext-quota-20261028/` returned GREEN_MASTER while the client
spun inside `memcpy` before any paging proof. It is explicitly **rejected** by the
paging checker. The failing ELF and ISO are retained there. Volatile byte copies
and fills fix `memcpy`/`memset`; optimized helper execution and code-generation
checks reject self-recursion. No pager ABI change was needed for this defect.

Evidence-checker mutation tests reject missing/split records, forged task/token/RIP
joins, frame reuse, wrong rights/pkeys, omitted zeroing, early completion, duplicate
leaf writes, wrong-task quota rejection, resurrection and absent successful
unrelated progress. Legal user progress before sext's own commit log remains valid.

## Acceptance evidence

All builds used the single sealed entrypoint through `master_runtime_gate.sh`.
Validation is local macOS/QEMU 11.0.2 x86-64 guest execution, not a completed remote
CI run. The accepted normal image has empty kernel and kaleidoscope rustflags.

| Evidence | Observed result |
| --- | --- |
| `/tmp/sexos-sext-verified-20261028/` sealed build and full 50-second master window | GREEN_MASTER; sext spawn/init/readiness required. |
| Same normal guest: paging checker | Four real missing-page instruction retries; exact token/task/page/frame/RIP joins; zero checks, data retention and live authority probes PASS. |
| Same normal guest: memory and scheduler ownership gates | Both PASS; immutable genesis/sealing and existing park/wake custody remain observable. |
| Compiled scheduler stack-release and timer-PKRU checks | PASS on accepted kernel. |
| Accepted kaleidoscope ELF | Optimized memcpy/memset self-recursion check PASS. |
| `/tmp/sexos-sext-quota-fixed-20261028/` declared quota build, full 50-second window | GREEN_MASTER and strict quota checker PASS: four mappings, fifth in-range fault rejected, no observed rejected-task resurrection, successful clock-domain IPC afterward. |
| `/tmp/sexos-sext-verified-host-20261028/` | All 20 registered host/static gates PASS; 218 Rust test executions, including repeated shared helper suites. |
| `/tmp/sexos-sext-final-python-20261028.log` | All 101 Python regressions PASS, including 13 paging-evidence mutation tests. |

The accepted host manifest binds 228 source/harness/dependency files by SHA-256.
Its contents were checked against the current files after execution. New suites
cover 13 fault-transaction tests, 12 live/historical forwarder/codec tests, 8 mapping
state/production-commit tests, 17 sext allocator/policy/bounded-record tests, and 4
optimized compiler-memory helper execution tests. Existing ownership, region,
storage, readiness and selector suites remain enabled. Mock TLB/transport tests
are explicitly distinct from the guest's real PTE installation and IRET retry.

Normal ISO SHA-256:
`4d1ab5a1e946b52c031edaa721c5f826f88af41778182c75e2667aef9cd3bc82`.
Accepted kernel SHA-256:
`35ea3124fc7c1fd355505431d332587f6c8d51990fc55fbb064d63b716621dc4`.
Quota-proof ISO SHA-256:
`05b5fb38491a7bd18fef7fb1493880d353cdc84f90ed68ab71e8bf613b13db18`.
Each accepted build directory retains its tested ISO, serial log, build log,
command and source/image provenance. The quota image predates the final existing
memory/USB diagnostic formatting changes; its paging mechanism/policy/probe is
unchanged apart from removal of the declared quota cfg in the normal image.

An intermediate normal image passed paging but failed an existing memory/USB
evidence row because timer text split a legacy marker. Required memory records
and USB diagnostics now use the bounded record transport too. That failed evidence
was not promoted to success or bypassed by weakening the scanner.

Fresh runs on that exact normal ISO also pass:

- All four USB lanes: tablet discovery/enumeration/input/contact, no device,
  keyboard forwarding and relative mouse forwarding. Complete summary and lane
  logs: `/tmp/sexos-sext-verified-usb-20261028/`.
- Two-boot persistence: both disk publications; Quil save/reload with matching
  242 bytes and fingerprint `0xdbc6d3de7f43121f`; disk-open/load/render observations;
  RSP0 and fault checks. Logs: `/tmp/sexos-sext-verified-persistence-20261028/`.
- All ten object lifecycle rows: create, duplicate-name rejection, listing,
  rename, protected system slot, reboot survival, delete, updated list, slot
  reuse and fault scan. Copied logs, final disk image and provenance:
  `/tmp/sexos-sext-verified-dynamic-20261028/`. The root ISO hash matched before
  and after this legacy gate, which uses that root image directly.

Both lifecycle boots use QEMU's default CPU and explicitly report PKU unsupported.
Each independently passes the same strict paging checker: four actual read/write
recoveries, retained bytes, authenticated capability denials and idempotent/conflict
probes. The normal master uses `max,+pku` and passes those checks too. This is
paired guest evidence for the implemented paging protocol across PKU settings;
it is not full native-code isolation or whole-system authority equivalence.

No runtime source changed after the final normal build. Reports and CI metadata
were completed afterward. CI now requires sext boot readiness, the strict guest
paging proof and post-build client memory code-generation checks in addition to
the existing checks. This is configured CI, not a claim that remote CI executed.
Prototype or GREEN_MASTER results alone were not treated as paging recovery proof.
The executable inventory remains 25 obligations: 8 open, 17 partial; none proven.

## Remaining work

The supported runtime slice is one bounded client range and registered main-task
faults on the BSP. It has no eviction, unmap/reclamation, client/pager restart,
frame-pool expansion, executable demand mappings, general task support, physical
AP scheduling or SMP TLB shootdown. Committed mappings remain reserved until
reboot, including after quota rejection. Device/IOMMU confinement, native WRPKRU
containment and privileged SyncCall safety remain separate obligations. Full
epoch/input recording and system replay remain open; local fault ledgers and
serial records are not a complete epoch engine.

No commit, push or canonical architecture edit is performed by this work.
