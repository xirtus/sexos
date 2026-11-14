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
with `exact_two_tick_envelope=false`. It establishes later continuation of that
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
