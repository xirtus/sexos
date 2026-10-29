# Scheduler ownership and stack release — 2026-10-28

The runtime now derives lifecycle, scheduling custody, pending wake, and fault
wait state from `TaskOwnership` in `kernel/src/task_ownership.rs`. This replaces
the bounded lifecycle-only wake repair described in
`SCHEDULER_WAKE_LIFECYCLE_2026_10_28.md`; that earlier helper's isolated tests
remain historical evidence, not the active runtime ownership implementation.

## Transition and publication contract

| Phase | Custodian and permitted next step |
| --- | --- |
| Constructed | Creator only; explicit activation after context initialization |
| Detached | Saved context has no CPU owner; eligible admission may reserve a token |
| Queued | Exactly one publication reservation or published queue token |
| Dispatching | Dequeue winner reserves execution for one core |
| Attached | Current task belongs to one core, including a retained blocked task |
| SwitchingOut | Outgoing core still owns context writes and physical stack use |
| Dead | Terminal; wake cannot create runnable custody |

Wake records a pending event independently of custody. Only Detached custody
can become Queued. A wake before park is consumed by park; a wake during
Attached or SwitchingOut never publishes the task. Dispatch and ordinary yield
preserve a pending event until an actual wait consumes it. Exit remains terminal
while outstanding queue or CPU custody is drained.

Admission reserves Queued with a CAS before queue publication. Full or Contended
publication rolls back to Detached without losing concurrent pending/exit state.
`Scheduler::retry_admission` scans 32 entries of the existing domain registry per
tick, covering each registered domain's main and trampoline task. These are all
current `Task::new` creation sites. Additional future task inventories must join
this retry mechanism before relying on deferred admission.

`Scheduler::tick` no longer publishes its outgoing task. It records an explicit
`pending_outgoing` pointer and keeps the task SwitchingOut while its caller saves
the complete context. `switch_to` disables interrupts, retains the next context
in callee-saved R12, pivots RSP onto the next task's saved frame, aligns call
scratch below that frame, then calls `scheduler_finish_switch`. The callback
checks that RSP is outside the outgoing stack and its saved frame is in bounds,
releases ownership, and attempts admission. It does not read the old context
after release. It never derives a Task pointer from a TaskContext address.

The callback runs before PKRU narrowing. The saved frame is reloaded afterward;
the 15 GPR pops, dummy slot, IRET frame and TaskContext offsets stay unchanged.
All callers retain the RSP0 rule `saved_frame_base + 168`. Syscall SFMASK clears
IF, DF, TF and AC at entry, before the GS/RSP transition.

The timer stub saves entry PKRU in its existing dummy error qword (GPR-frame
offset 120) before opening kernel access. If the handler returns, including
NoRunnable and kernel-mode timer returns, the stub restores that exact value
before user GS/GPR restoration. The switch path still discards the same dummy
slot, so this changes no offsets or frame size. Both operations are guarded by
PKU availability. This fixes the prior normal timer-return path leaving PKRU=0;
it does not establish containment against untrusted native WRPKRU instructions.

## Real self wait and faults

Self-only syscall 53 checks the caller's inbox/reply queue before parking.
Successful Domain payload publication records a wake on its receiver. The
pending handshake closes the empty-observation/park race. An attached blocked
task with no replacement retains its current-task/current-PD binding and saved
frame; the kernel waits using STI/HLT and resumes locally only after consuming
its pending wake, or switches when a concrete replacement becomes available.

Fault waits carry a separate fault-wait bit. Generic reply, Domain or IRQ wakes
cannot resume an unresolved faulting instruction. Rejected pager dispatch
terminates the faulted task. An accepted future pager transaction requires a
separate authorized completion protocol; no generic wake grants that authority.

## Evidence and limits

`scripts/task_ownership_host_gate.sh` passes 22 production-helper/queue tests,
including 517,744 finite operation traces and 20,000 concurrent context
transfers. These establish tested atomic and queue properties, not x86 stack
departure. `scripts/check_scheduler_switch.py` checks the built kernel's actual
instruction ordering; sealed-image QEMU checks must establish runtime execution.

`scripts/timer_pkru_return_gate.sh` inspects the built timer stub; its checker has
eight passing mutation tests in `scripts/tests/test_timer_pkru.py`. Before the
stub repair it rejected sealed kernel SHA-256
`658fcba3d6c0f265e1ef4e387fd0f95afa2eef5ea6a95a2ec50ead4ed81c4417`
with `expected one entry PKRU read; found 0`, captured at
`/tmp/sexos-timer-pkru-red.json`. The rebuilt artifact must pass the same gate.

Runtime observations use `[scheduler.ownership] event=` with `park`,
`detached_wake`, `post_stack_release`, `local_resume`, and `user_resumed`.
`user_resumed` is emitted only at the next user-origin syscall/timer entry after
a park, rather than merely when the task is selected.

The optional `scheduler_wake_proof` cfg is supplied by the declared sealed build
spec. On the first BSP blocked self-wait it temporarily retains existing Queued
tokens, keeping their custody exclusive. A real kernel timer interrupt records
a wake while the last task remains Attached. Local resume asserts preserved
ownership and restores every retained token once. Its three required markers
are `scheduler.proof.last_runnable.begin`, `.timer_wake`, and `.pass`. Overflow,
wrong ownership or failed token restoration fail closed. Normal builds omit
this token retention and synthetic timer wake.

Only the BSP currently schedules; AP startup is parked and TSS state is shared.
Host concurrent tests and QEMU `-smp` configuration do not prove multiple live
kernel scheduling lanes. A remote wake of an attached blocked owner sets pending
without an IPI, so progress depends on the owner's periodic timer or another
interrupt. Full AP bring-up and explicit kick policy remain separate work.

Sealed proof/normal build hashes, QEMU logs and compiled-gate results belong in
the parent integration report. Source was frozen before those runs; this note
does not substitute for their final acceptance result. The first proof run at
`/tmp/sexos-owner-proof-20261028/serial.log` exercised task 10 with 13 retained
tokens: begin at line 1681, timer wake at 1687, preserved local resume at 1689,
and actual user re-entry at 1690. That proof predates the symmetric timer PKRU
return repair; the final sealed image must be tested again after the repair.
