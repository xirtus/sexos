# Scheduler wake lifecycle containment

Date: 2026-10-28. Scope: SASOS task lifecycle only, not execution ownership.

## Failure and bounded repair

Previously `unpark_thread` unconditionally stored Ready. An interrupt or reply
could wake an attached Running task, changing it to Ready. On the next switch,
`tick` detached that task but requeued only outgoing Running tasks. The Ready
task then had no current owner or queue token. The same unconditional store
changed Exited back to Ready, and converted unknown lifecycle values to Ready.

`kernel/src/scheduler_wake.rs::wake_blocked` now performs one strong atomic CAS
from Blocked to Ready. Other states remain unchanged. Successful CAS is the
lifecycle transition point; an exit that precedes it causes failure, and an exit
that follows it wins. `scheduler.rs::unpark_thread` guards null and calls the
helper. Compile-time assertions pin the helper's Ready/Blocked numeric values
to `TaskState`. No task is queued or detached by this helper.

The repair changes no Task/TaskContext fields, switch assembly, IRET offsets,
RSP0 arithmetic, queue algorithm, current-task swap, or NoRunnable ownership.
It introduces no memory mapping, capability, address-space, or pager policy.

## Red-first evidence

Before the runtime edit, the exact `unpark_thread` body and `TaskState` enum
were extracted from the working production source and compiled in a host
harness with a minimal Task containing a borrowed AtomicU32. The extracted body
was:

```rust
pub fn unpark_thread(task_ptr: *mut Task) {
    unsafe { (*task_ptr).state.store(TaskState::Ready as u32, Ordering::Release); }
}
```

`tests/scheduler_wake_lifecycle.rs` ran unchanged against that body: **2 passed,
6 failed** (Running preservation twice, terminal exit three times, and unknown
state preservation). Evidence from the session:

- `/tmp/sexos-scheduler-wake-baseline.rs`: extracted production body and harness.
- `/tmp/sexos-scheduler-wake-baseline-body.txt`: exact original body.
- `/tmp/sexos-scheduler-wake-red.log`: failing assertions and counts.

This tests the production function's state operation with a stub Task; it is
not a complete compiled-kernel test. After repair, the same eight assertions
compile directly with the actual production helper and pass **8/8**.

## Ownership model and remaining work

`tests/scheduler_ownership_model.rs` contains **12 deterministic model tests**:
eight historical counterexamples and four reference-protocol checks. The
`baseline_*` names deliberately refer to the pre-repair store, not the patched
helper. Their passing results reproduce those deficiencies; they do not prove
the current kernel correct. The reference test enumerates all 21 placements of
two wakes around save/stack-release/admission operations in a serialized model.

The future ownership invariant for each task is:

```text
queue_tokens + dispatch_reservations + attached_or_switching_owners <= 1
dispatchable => context_saved AND previous_stack_released
constructed/dead => no execution token
```

Wake must publish at most one retained pending event, separate from execution
ownership. A future protocol needs Constructed, Queued, Attached, SwitchingOut,
Detached and Dead distinctions. Current lifecycle Ready is insufficient.

Still unresolved:

1. Wake before park has no durable pending token. Preserving Running does not
   prevent a subsequent Blocked store from losing the event.
2. A detached Blocked task changed to Ready receives no queue token. A wake
   while an attached task is Blocked can likewise leave Ready detached when
   the next switch skips requeueing it.
3. The page-fault NoRunnable path waits for Running, while wake produces Ready;
   kernel-mode timer interrupts skip scheduling. Pager completion/local resume
   needs a separate authorized protocol.
4. `tick` publishes the outgoing task before handlers save its context.
   `yield_and_switch` saves first but publishes before leaving its stack.
   Neither queue correctness nor clearing current_task establishes safe remote
   execution. Save plus post-RSP-transfer release discipline is required.
5. Constructed signal trampolines have Ready lifecycle and zero entry RIP.
   Generic Ready admission must not activate them.
6. Raw task-pointer lifetime remains a caller obligation. Null containment
   does not establish object validity or future reclamation safety.

Direct wake enqueue and a global scheduler redesign are intentionally outside
this bounded repair. Full/Contended queue failures must eventually retain a
pending token for retry under the complete ownership protocol.

## Verification

- `./scripts/scheduler_wake_host_gate.sh`: actual helper **8/8**, ownership model
  **12/12**. Host-only unit gate, not another kernel/image build route.
- `./scripts/scheduler_queue_host_gate.sh`: actual queue **8/8**.
- `./scripts/scheduler_no_runnable_ownership_gate.sh`: PASS.
- `./scripts/rsp0_regression_gate.sh`: static checks PASS.

Host atomic tests and serialized reference models do not prove weak-memory or
real SMP execution safety. The parent integration lane owns the sealed build
and QEMU checks; this handoff makes no new runtime or pager-recovery claim.
