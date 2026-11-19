# Passive diagnostic wait observer

This optional witness addresses independently sampled kernel expiry and Net
return logs. The original park/expiry/return and reviewed UP return-envelope
paths remain unchanged. It does not infer timer causation from elapsed time.

Only `sexos_input_payload_proof` compiles the observer and its call in Shell's
existing loop. It reads at most once per 250 kernel ticks, at most 512 samples,
and creates no wake, wait, IPC, application, capability, history or wire traffic.
An idle Shell is never woken to sample. Normal compilation omits the module.

Each sample reads the kernel tick before any observation, reads the peer state
through Shell's existing Net route (`web::SHELL_SLOT`, 262), reads PUBLIC
SYS_STATS87's domain13 counter, reads that peer state again, then reads the tick.
Successful metadata requires both peer snapshots to be epoch1/Live and the
service-row task field to be0. The route is already granted to Net by BOOT;
peer_state's kernel lookup checks the genuinely held Domain capability.

The source-bound checker requires the reviewed original causal source/kernel
cohort plus exact observer, Shell, SDK stats/peer/route, CPU counter, core-local,
restart, resource and catalog sources. Net's unique BOOT spawn, initial restart
registration and handoff establish its fixed main task. Net has no program row
or worker authority. Counter increments use the current domain, so Shell's
queries cannot increment Net's counter. Service restarts reuse the task and
preserve counters; epoch1 checks therefore are mandatory.

Both selected reads must appear after a complete sampled Net expiry and have
`before_tick >= expiry.now`. On the reviewed UP interrupt path the tick is
advanced before deadline polling, and Ring3 cannot observe that tick until the
ISR finishes; this lower-bound fence excludes reads that preceded expiry but
were logged afterward. A later strictly larger Net counter establishes fresh
Ring3 syscall entry after that expiry. Missing expiry, unknown source, SMP,
malformed/duplicate/gapped samples, resets, non-live/restarted snapshots and
observed Net fault/worker/lifecycle ambiguity fail closed. The exact positive
BOOT restart-registration line is not mistaken for a runtime restart.

The evidence type is `source_bound_up_expiry_later_ring3_syscall_continuation`,
with `exact_two_tick_envelope=false`, in `passive_continuation_joins`.
The legacy `timer_user_joins` remains only observed/return-envelope joins. It establishes later continuation of that
first-epoch BOOT task, not the exact two-tick return envelope, a wake reason from
the syscall ABI, fault-free execution between snapshots, NIC IRQ behavior,
physical/SMP behavior or a general Rust verifier. Recoverable fault history is
not latched by peer_state; the checker rejects observed faults and non-live
snapshots without claiming an absence of every transient between them.

Host tests compile the actual observer with diagnostic cfg and without it,
verify throttling/budget/epoch barriers and normal omission, and compile the
actual kernel counter/read implementation with only its TSC hardware intrinsic
stubbed on the ARM host. Synthetic proof negatives cover pre-expiry reads,
postdated logs, zero/early/pre-handoff expiry, wrong/missing/duplicate identities,
no counter growth, resets, malformed samples, epochs/routes/tasks and source
mutations. These are host/control results; acceptance still requires independent
review and an actual newly built diagnostic cohort. No kernel source is edited.


## Reviewed Shell exit-subject correction

After the sealed c84dc5f8 runtime cohort, commit 0b9364569 adds one pure
`launch_exit_subject` predicate and changes only the OP_EXIT status-mutation
condition to compare both the child's domain and task. This prevents a delayed
old-task message from setting a reused domain's status. It does not change the
observer call, sampling, route262, Net service identity, stats accounting, clocks,
waits, wakeups or any kernel source. The complete Shell source fingerprint is now
`b27da0f06b6c81b624a2ad78f35fb9e8a00738c60bc3917bd06df7aa9d84c97a`;
all other observer and causal-core pins remain unchanged. The actual production
predicate is compiled in the host negative test. This review permits the new
source cohort; it does not carry the old runtime results onto a changed image.
The c84 proof and its own frozen checker retain their original fingerprints.
Shell main is not an entry in spec/abi_inputs.sha256; no ABI pin is fabricated.


Validation after the correction: all 1,311 Python regressions pass in167.161s,
including the compiled production exit predicate and passive-observer negative
controls. All85 existing ABI input digests remain exact; no ABI manifest change
is necessary. The new Shell source still requires its own guest qualification.
