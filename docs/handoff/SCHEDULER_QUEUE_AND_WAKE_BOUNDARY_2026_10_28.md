# Scheduler queue repair and remaining wake boundary

Date: 2026-10-28. Baseline: `4a040a86`.

## Delivered

`scheduler.rs` now uses the bounded pointer queue in `scheduler_queue.rs`.
The old queue had one unreserved producer position: simultaneous producers
could overwrite one slot and publish the same bottom increment. It also silently
ignored insertion when full. Queue insertion now reserves a position with CAS,
then publishes that slot with a release sequence; consumption acquires the
publication and releases the slot only after reading the value. This protects
slot reuse when a consumer pauses across queue wrap.

`try_push` rejects null and returns `Null`, `Full`, or `Contended` without
reserving a position on error; the caller retains the pointer. Reservation
attempts are bounded by queue capacity. Existing `push` call sites use an
explicit panic on error, so saturation no longer silently strands a runnable
task. This is fail-closed containment, not scheduler overload recovery. A future
wake owner must retain and retry requests using `try_push`.

The queue is MPMC, but this does **not** make task scheduling SMP-safe. It
transfers pointer values only. It does not validate pointer lifetimes, suppress
duplicate submissions of the same pointer, or grant execution ownership.
Null from `steal` can mean empty, contention, or an unpublished reserved head;
it is not proof of quiescence. A stalled producer can temporarily block the
queue head. The finite-width sequence comparison assumes no individual
operation survives half the `usize` position space of progress; ordinary
counter wrap is supported and tested.

TaskContext, switch assembly, IRET offsets, RSP0 arithmetic, current-task swap,
and NoRunnable ownership were left unchanged. No paging or authority model was
changed. Each queue slot now has a sequence word in addition to its pointer;
the 128 x 512 queues use approximately 512 KiB more static memory on x86_64.

## Why direct wake enqueue remains unsafe

1. `tick()` swaps `current_task` and enqueues a Running outgoing task before
   the timer/page-fault handler saves that task's TaskContext. Another core can
   steal the outgoing task while its context and kernel stack are still in use.
   For a blocked outgoing task, observing absence from `current_task` likewise
   does not prove that its context has been saved or its stack released.
2. An attached blocked task cannot be queued simply because a wake CAS changes
   Blocked to Ready. It remains the current execution owner, including when
   NoRunnable preserves it. Queuing it duplicates execution ownership. Scanning
   current_task pointers is insufficient: ownership changes race with the scan.
3. The page-fault NoRunnable path waits for Running, but `unpark_thread()` writes
   Ready. Kernel-mode timer interrupts skip scheduling. Therefore even a safe
   detached wake queue would not resolve the attached idle wait by itself.
4. `yield_now()` and `yield_and_switch()` publish a task before clearing
   `current_task`. The latter has saved its frame, but still executes on that
   kernel stack until `switch_to` changes RSP. A remote consumer must not reuse
   that stack during the interval.
5. Signal trampoline tasks are created Ready with entry RIP zero. Treating every
   detached Ready task as newly runnable would activate unfinished trampolines.
   Ready alone cannot distinguish an admitted task from a constructed task.
6. Current callers use raw Task pointers from registries. Tasks are currently
   leaked through `Box::into_raw`; future reclamation needs generation/lifetime
   protection. Heap-range checks do not establish object validity.

These are source-visible correctness blockers, not an approval requirement.
`unpark_thread()` remains unchanged; detached wake, repeated wake suppression,
and Exited-task wake rejection are **not fixed** in this change.

## Required follow-up protocol

Define a per-task execution owner distinct from lifecycle state, with explicit
constructed, queued, attached, switching-out, and detached ownership. Wake
publication must record at most one pending token without granting execution
ownership. Attached/switching-out wakeups remain pending for their owner;
detached wakeups can be admitted once, with a retained token if the queue is
full or contended. An Exited task rejects wakes. Publication before parking
must be covered by the park/wake handshake rather than erased by a later store.

Release of an outgoing context must occur after its last save **and after
leaving its stack**, with release/acquire ordering to its next dispatcher.
This needs a defined switch boundary and coordinated timer, page-fault, yield,
and bootstrap callers; a pre-switch Rust enqueue cannot prove that condition.
TaskContext assembly layout and RSP0 must remain pinned by existing gates.
The attached NoRunnable path separately needs a local resume/idle mechanism
that consumes pending wakes without changing the preserved ownership contract.
Until this protocol is implemented, queue correctness is not a claim of SMP
execution safety or pager recovery, even if current runtime boots use one core.

## Verification

- `./scripts/scheduler_queue_host_gate.sh`: PASS, eight tests of the actual
  kernel queue implementation: null rejection, full queue and retry/FIFO,
  explicit saturation panic, usize wrap, delayed producer publication, delayed
  consumer slot release, 80,000 values across four producers/four consumers
  checked exactly once, and payload publication visibility.
- `./scripts/scheduler_no_runnable_ownership_gate.sh`: PASS.

The host gate is a unit-test harness only, not a kernel/image build route.
The parent integration lane runs the sealed `make full` and QEMU/runtime gates.
Host stress tests are not an exhaustive weak-memory model checker. Future wake
integration must exercise repeated/concurrent wake, wake before park, attached
and switching-out wake, detached wake, full queue retry, exit races, duplicate
suppression, and context-save/stack-release races on multiple cores.

## Parent integration closeout (2026-10-28)

Sealed build and 40-second QEMU master gate: **GREEN_MASTER**. All four USB
lanes and the strengthened two-boot persistence/rendering gate pass on the same
image. See [the execution report](SEXOS_RELIABILITY_ROADMAP_2026_10_28.md)
for source/image provenance, commands, evidence paths and remaining limitations.
