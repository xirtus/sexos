# Bounded sext paging mechanism

Status: two-client reclaim and task-exit cleanup candidate implemented; host mapping
gate fourteen tests pass. Parent
integration owns sealed build and live fault/recovery evidence. Host mocks below
are not hardware proof. Preserve the canonical architecture: one GLOBAL_VAS,
immutable BOOT_DAG capability genesis, sex-pdx authority, optional PKU enforcement.

## Authority and mechanism

Boot grants sext one eight-frame pool at slot34 and two mapping capabilities at
slots36/37. Each client receives its own demand-range capability at slot35 and a
Domain route to sext at slot2. The pool covers eight 4 KiB frames shared between
two disjoint eight-page reserved ranges. Each mapping capability bounds its
target to four simultaneous pages; sext also applies its own four-page policy.
Domain14/key14 is sext; primary kaleido is domain15/key15 and second client
sexbell is domain10/key10. Runtime decisions use actual capabilities and their
boot-bound identities rather than hardcoded numeric caller checks.

The frame pool has a deliberate RW/NX alias under sext's key. The pool's physical
addresses are never disclosed by paging info or commit. Sext chooses an opaque
frame index, zeroes the selected page through its authorized pool alias, and
requests a leaf transition. The retained pager alias is explicit authority over
those frames; this design does not isolate clients from a hostile pager.

`boot_memory::init_paging` reserves backing using the existing whole-range boot
exclusion mechanism and zeroes it before exposing it to sext. It creates the
target page-table infrastructure through temporary boot mappings, unmaps each
leaf and records the existing leaf entry address. Every leaf is asserted zero
before grants are published. No page-table allocation is needed during a fault.

`kernel/src/paging.rs` never calls a frame allocator, VA allocator, map_to,
map_physical_range, capability insertion, or memory-zeroing operation. The narrow
runtime operation validates an invoked mapping/pool capability, claimed pending
fault token, target identity, reserved page, selected frame index and requested
rights. It verifies sext left the frame zero, records unique ownership, writes
the existing leaf PTE and invalidates that VA locally. The PTE uses the target's
boot pkey and requested authorized read/write permissions, always USER and NX.
Current task execution is on the BSP; this is not an SMP shootdown protocol.

## Runtime ABI

| Syscall | Inputs | Outputs |
| --- | --- | --- |
| 56 commit | RDI mapping slot, RSI pool slot, RDX frame index, R10 token, R8 rights | RAX status |
| 57 query | RDI mapping slot, RSI token | RAX status, RSI fault page, RDX fault error, R10 target PD, R8 allowed rights |
| 58 reject | RDI mapping slot, RSI token | RAX status |
| 59 info | RDI capability slot | RAX status, RSI base VA, RDX bytes, R10 target PD or zero for pool, R8 rights |
| 60 reclaim | RDI mapping slot, RSI pool slot, RDX frame index, R10 original fault token | RAX status |
| 61 cleanup | RDI mapping slot, RSI pool slot | RAX status, RSI detached frame bitmap, RDX exact target task ID |

All slots are decoded at register width. Invalid query/info outputs are zero.
FramePool, PagerMapping and DemandRegion are distinct capability kinds; existing
54/55 region queries do not accept them. Statuses are OK0, INVALID1, RETRY2,
DENIED3, EXHAUSTED4 and LIVE5 (valid cleanup authority, target still alive). Existing raw allocation and mapping syscalls remain denied.

Authenticated query metadata includes a client's pending fault outside the
mapping extent, allowing sext to reject an unsatisfiable request. Query grants no
mapping right; commit still enforces the exact reserved extent. Rejecting such a
query in the kernel instead would strand the client when sext correctly ignores
untrusted or stale notifications.

## Fault transaction and uniqueness

The scheduler lane owns pending token registration, saved context, claim/rollback,
completion and rejection. The mapping mechanism invokes its trusted interfaces
only after capability validation. A valid request targets an ordinary user
not-present read/write fault (error4/6); protection, execution, reserved-bit and
PKU violations cannot become demand mappings through this mechanism.

`paging_state::MappingLedger` records each frame's exact token, pager, target,
range/page index and rights. Frame uniqueness is global; page identity includes
its target/range; quota counts are per target. Equal page indices in distinct
client ranges are independent. A token cannot be rebound. No executable page,
mapping-right upgrade or arbitrary target-VA operation is offered.

Before publication, validation/dirty-frame/quota/PTE conflicts leave the leaf and
ledger unchanged and roll back the fault claim. After publication, the frame
remains owned regardless of task exit or completion contention. Every completion
failure returns RETRY, even if the fault helper reports INVALID or DENIED: the
caller must retain the same frame and retry the same tuple without zeroing it.

An installed but unfinished exact retry completes fault ownership. A finalized
exact retry returns OK without touching contents, PTEs or wake state. Conflicting
duplicates return DENIED. Finalized live records retain idempotent results after a newer fault, including
when the newer fault has already been reclaimed.

Reclaim accepts only an exact finalized binding under the invoked mapping and
pool capabilities. The leaf must match its physical frame, USER/NX/RW and pkey;
only hardware Accessed/Dirty bits may differ. Under one transition lock it clears
the leaf, invalidates that VA locally, then retires ownership and makes the frame
eligible for a later commit. Syscall entry masks IF and APs remain parked; no
remote TLB or concurrent-user-execution guarantee is claimed. Sext must zero a
reused frame through its retained authorized alias before committing it again.

Each frame retains its last retired binding even while newly occupied. An exact
retry of that reclamation returns OK without touching the new owner's leaf. A
later reclamation overwrites the bounded tombstone; an older retry then returns
DENIED without side effects. Historical successful retry results are not stored
without bound. A per-range retired-token highwater rejects retired commit tokens
even if redirected to another frame. This relies on the enforced single main
faulting task per range and monotonic fault tokens; it must be redesigned before
supporting multiple concurrently pending faulting tasks in the same range.

Client release uses authenticated OP_RELEASE0x104 with page and a nonzero
monotonic request ID. Sext owns routing, its exact request/result cache and reply
policy. It retains frame custody on RETRY or unknown status and releases it only
after syscall60 returns OK. The kernel does not authenticate release IPC or choose
which allocation the userland pager should release.

## Terminal task cleanup

The bounded cleanup operation is userland-requested through the existing boot
mapping and pool capabilities. No new capability, frame policy, allocator, page
table, or address range is created. A boot-registered scheduler watch pins the
exact main-task pointer. Its validator requires terminal Exited/Dead ownership
with no CPU owner; an exited Attached/SwitchingOut/queued owner receives RETRY.
A live target receives LIVE5. Any signal trampoline must also be inert or dead.
The scheduler's post-stack-switch release establishes the required detachment;
terminal intent alone never authorizes reclamation.

Each live mapping now records the original fault's task ID. Cleanup verifies that
all target bindings belong to the exact dead task/pager/range, preflights all
owned PTE identities (ignoring only hardware A/D), and requires unowned leaves to
be absent. Only after every preflight succeeds does it retire that dead task's
fault metadata. This retirement accepts a published Claimed transaction whose
mapping completion was uncertain. It can return RETRY before any leaf changes.

Cleanup then clears and locally invalidates every planned leaf while retaining
all ledger ownership. After the last flush, it retires all bindings, including
published but unfinalized bindings, permanently closes the range and caches the
frame bitmap/task ID. There is no fallible external operation between the first
leaf write and permanent closure; invariant failures panic instead of returning
a misleading no-effect error. The scheduler watch is acknowledged afterward.
An exact cleanup retry returns the cached result without touching frames or PTEs,
including after another client has reused those frames. No respawn or reopened
range is supported in this boot-only model; later query/commit is denied.

Sext applies successful cleanup once to that target's own Reserved, Committed or
Reclaiming records. The returned bitmap is evidence, not permission to free
arbitrary current allocations. Unpublished reservations can also be released
because the range is permanently closed and its task cannot resume. Until OK,
uncertain commit/reclaim ownership remains retained. Scheduler exit messages are
hints only: the cleanup syscall repeats authoritative validation.

## Tests and review correction

`bash scripts/paging_mapping_host_gate.sh` passes fourteen tests: thirteen exercise
the actual pure validation/ledger; one runs the complete production paging module
with real test memory for frames and leaf entries. Capability table, fault
transport and TLB instruction are explicit host mocks. It covers malformed slots,
rights and kinds, dirty-frame rejection without kernel zeroing, both clients'
independent page indices/quotas, global frame exclusivity, target-pkey/NX bits,
rollback, completion retry, and unchanged contents on exact commit retry.

Reclaim coverage includes unfinished-commit denial, changed physical/pkey/access
bits, A/D allowance, no mutation on wrong token/capability, and a callback at the
actual production flush call observing an absent leaf while reentrant operations
still receive RETRY from the held transition lock. A source-order assertion checks
clear-before-flush-before-retire. Reuse coverage includes stale commits into other
frames, old reclaim after reassignment, tombstone replacement, older still-live
commit replay, quota recovery and 1,000 alternating-client reuse cycles. These are
host mechanism checks; the TLB mock does not establish hardware invalidation.

Sensitivity checks used temporary source copies, preserving the candidate:
restoring the old global page-index collision rule failed the independent-client
test (zero passed, one failed, unexpected DENIED3); removing the exact PTE guard
failed the complete production-module test (zero passed, one failed, observed OK0
instead of DENIED3). The unmodified fourteen-test gate passes. Cleanup-specific temporary mutations
also fail the actual-module test: removing full-leaf preflight returns OK instead
of DENIED; removing permanent closure loses the cached bitmap on retry. Each
mutation produced zero passed/one failed without changing production sources.

The actual-module test additionally covers live cleanup refusal, exit-before-stack
release RETRY, wrong-task identity, a read-only unfinished publication, a mismatch
in a later leaf aborting before the first clear, an unexpected unowned PTE,
dead-fault retirement contention without changes, and callbacks at both cleanup
flushes observing the held transition lock. It verifies permanent query/commit
closure, other-client isolation, cached cleanup after cross-client reuse, and an
empty target with an unpublished pending fault. The task validator and fault
retirement are explicit host mocks; scheduler production-helper tests and guest
post-stack-release evidence are separate proof obligations.

Independent review caught an initial ABI error: a failure returned by completion
after PTE publication was forwarded as a pre-publication-looking denial. A
temporary snapshot restoring that behavior failed the production-module test
(observed3, expected2; zero passed/one failed). The repaired production code
normalizes all post-publication errors to RETRY and the preceding eight-test suite passed. The current fourteen-test suite retains
that regression.

The preceding memory-authority gate still passes34 tests. These results do not
prove live #PF frame preservation, actual ring3 execution, correct hardware PTE
visibility, scheduler resumption or continued unrelated-domain operation. Those
require the fresh sealed image and explicit parent runtime checks. Remaining
native WRPKRU/privileged SyncCall and DMA containment limitations also remain.
