# Kernel pass 2: from working to world-class (2026-11-20)

Requested by Andreas on 2026-11-19: once the failing-PR sweep settles, do another
ambitious pass over the microkernel, because SexOS replaces Linux as his daily driver
soon. This document is that pass's plan. It builds on the first plan
(`KERNEL_STUDY_AND_REDESIGN_2026_11_17.md`, PR #405), keeps its budget (22,700
non-blank kernel lines, not raised) and its rules (no Linux drift, shims before flag
days, merge on green).

Basis: master at `74b9d73d` (2026-11-19 18:10 UTC). The kernel is **21,560** non-blank lines, so
1,140 are free.

## 1. Short answer

The first plan built most of the new machinery. The machinery is **not yet the system**.
The kernel now has two IPC systems, two authority models and two ways to deliver
lifecycle events, and every bug that cost CI time today lives in the *old* half:

- **The Files reply race.**
  - Files overwrote a program-load answer with a settings-poll answer.
  - Cause: the old IPC has one tagged reply channel per caller, not one per call.
- **The exit-notice storm.**
  - ~150 `[app.exit.notice]` resends arrived while Display was busy.
  - Cause: the kernel delivers lifecycle events as messages into a 256-slot ring and re-sends every 16 ticks until each of six holders acks.
- **Display's restart replay.**
  - A late `OP_APP_REBORN` ack for a dead run moved the kernel on while Display was still mid-cleanup.
  - Cause: lifecycle messages carry no run generation the kernel enforces, so each server must spot stale ones itself.
- **IPC sleep misses.**
  - About 20% of cross-core waits fall asleep, and their answer then queues behind whatever else runs on that core.
  - Cause: the scheduler has no direct cross-core handoff. #468 adds one.
- **"Poll everything on tick".**
  - The 1 kHz tick runs app-exit polling, service restart, IOMMU, ACPI, crash log and paging-exit watches with IF masked.
  - So the kernel can never go tickless (battery), and the housekeeping cost lands on whatever IPC trip is unlucky.

So pass 2 is about **finishing the move**, not new inventions:

1. **One IPC.** Old calls become shims over endpoints, and the mailbox is deleted.
2. **Events, not polls.** Lifecycle, restart and device events go through notification objects and a per-core deferred-work list.
3. **The scheduler that seL4 MCS and Zircon have:** direct handoff on every core, scheduling contexts with donation, tickless idle on TSC-deadline.
4. **Race hunting as a first-class gate.** A kernel jitter mode plus nightly runs make flakes like today's show up on the branch that causes them, not on master a day later.
5. **Real-silicon readiness for the x17:**
   - x2APIC.
   - TSC-deadline and CPUID-15h clocks (no PIT dependence).
   - SMAP with caller-keyed user copies.
   - IBPB.
   - INVPCID.
   - Microcode-aware mitigations.

Done as listed in §5, the kernel ends this pass **smaller** than today: the old mailbox,
credits, rings and the polling table (~1,500 lines) go out, and ~1,000 come in.

## 2. What today's bugs taught us

| Bug (where seen) | Mechanism that allowed it | Kernel fix in this pass | Stream |
|---|---|---|---|
| Files reply race: Shell stuck in `PROGRAM_READING`, `chello` spawn timeout (master run 44565909758; fixed in Files by #454) | One owed reply per (server, caller) in the old IPC. A server that defers work must queue replies itself; one forgotten queue loses an answer. | **One-shot reply capability per call** (IPC v2 `ReplySlot`). Two calls can never share a reply. The server holds N live replies, one per deferred request. | K1 |
| Exit-notice storm: ~150 `[app.exit.notice]` lines while Display retires a row (same run) | Lifecycle events are `IpcCall` messages in the holder's 256-slot ring, re-sent every 16 ticks per holder until acked (`app_resources::poll_exit_notifications`). | **Lifecycle notification word per holder.** The kernel sets the row's bit once (level-triggered), wakes the holder once, and the holder reads and clears pending rows. No resends, no ring slots, no loss under load. | K2 |
| Display restart replay: 29/31 acks, REBORN dropped, row stalls (Programs job, 3 flakes on #411; fixed in Display by #454) | The REBORN/EXIT protocol is a six-party ack quorum on messages without an enforced run generation. A late ack for run *n* counts toward run *n+1*. | **Acks name the run generation**, and the kernel refuses a stale one. The rebirth commit becomes one kernel state transition per holder (`Exited(n) → Forgotten(n) → Live(n+1)`), not a bit mask re-sent on a timer. | K2 |
| IPC sleep misses (~20% of 4-core waits) and the 4-core p99 tail (#417, #429, #468) | The cross-core answer wakes the partner into its core's run queue behind the current task. Placement is fixed by pd id, so hot pairs straddle cores. | Cross-core **handoff slot + preempt** (#468) for every wake that answers a call, **scheduling contexts with donation** so the server runs on the caller's budget and priority, and **pair affinity** in placement. | K3 |
| 4-core crash-proof boot exited at ~9 s (#449, earlier head) | Unknown. That run was cancelled before its log was read. | The **jitter build** (K5) is meant to reproduce this class. If #449's current run repeats it, the NVIDIA thread captures the serial tail and K5 adds it as a regression. | K5 |
| Self-wait pins block every scheduler edit (auto-mode refuses to self-approve pins; PRs conflict on pins) | Correctness is enforced by hashing source, so any edit needs a human re-pin. #436 narrowed this to wait functions; #454 makes pins union-merge. | Replace pins with **checked properties**: the Kani/loom models from phase 7 cover the wait state machine, and the boot proof checks behaviour, not source bytes. Pins stay only where no model exists yet. | K6 |
| Today's CI cost (600 job-minutes per run, freezes) | Every gate boots TCG for minutes, and flakes force re-runs. | Fewer flakes (K1/K2/K5) plus the KVM lane as the default for kernel gates. TCG becomes a nightly tripwire. | K5 |

The pattern: **every bug lived in a server-side workaround for something the kernel did
not express.** These were reliable delivery over a lossy ring, reply bookkeeping,
generation checks and liveness checks. The first plan moved the mechanisms into the
kernel (IPC v2 #410/#421, authority v2 #407, generations), but no service uses them
yet.

## 3. Where the kernel stands (first plan, phase by phase)

| Phase (2026-11-17 plan) | State on master | What is missing |
|---|---|---|
| 0. Real numbers | Done (#414, #418). KVM lane in CI, medians of 3. | KVM lane does not run `ipc2` or the 4-core latency gate. |
| 1. Clean and audit | Mostly done: audit #409/#437/#447/#462 merged; dead capability kinds gone (#407); ARCHITECTURE.md rewritten. | −800..−1,200 lines target **not met**: the kernel grew 20,238 → 21,560. The tick still does housekeeping. S11 (SMAP) and S14 (IBPB) are open. |
| 2. IPC v2 | Objects (#410) and syscalls 105/106 (#421) are live. | **No service uses them.** v2 is ~25x slower than the old path on 1 core (a wake without handoff, #426 investigating). 100/101/28/29 are not yet shims, so nothing has been deleted. |
| 3. Authority v2 | Inline table, generations, derive/install (#407). | Files, Collar and program rows still keep their own grant tables. Files still makes liveness syscalls per request. |
| 4. SMP | Cross-core spin (#429), handoff slot (#468 open), deadline class (#402). | Placement is still `home_core(pd)`. Global locks remain: `APPS`, `program_tenancy::POOL`, `program_workers::TABLE`, `GLOBAL_VAS`, `views::VIEWS`, `crash_log::RING`, `iommu::STATE`. No scheduling contexts, no tickless idle. The 4-core latency gate is still not required. |
| 5. Views + PCID + XSAVE | Views + PCID (#411), XSAVE/AVX (#408), rows moving to key 8 (#465 open). | No per-organ views without PKU; scaling of 8 programs on 4 cores is unmeasured; INVPCID is unused. |
| 6. PKU gates | P6.2a merged (#448); P6.2b-d in #466 (SYS_GATE 111, route binding, callee unwind). | Nested calls; a KVM-with-PKU proof (needs #465); moving real hot pairs (Files→SexDrive block reads) onto gates. |
| 7. Assurance | Models (#412): proptest/loom/Kani over capability table, IPC v2, queues, WRPKRU scanner; nightly fuzz. | Models cover v2 objects, not the scheduler's wake/park paths that flake. No timing-perturbation testing. |

Numbers (KVM, GitHub runners, cycles per round trip, best recent):
- Old path, same core: ~10-20k.
- Old path, 4 cores: ~68k (#429; was ~102k).
- IPC v2, 1 core: ~720k on TCG (the stall).
- seL4-class target: ≤1,000 same core.

**The single biggest performance lever left is moving services onto v2 with a working
fastpath.**

## 4. What world-class means for us (gaps against the reference systems)

What each reference system does that we don't yet:

- **seL4 MCS:**
  - IPC fastpath ~250-400 cycles.
  - Scheduling contexts and passive servers.
  - Notifications for IRQs.
  - Reply objects.
  - We have the objects but not the use, and no scheduling contexts.
- **Fuchsia Zircon:**
  - Ports and signals (one wait for many events).
  - Per-object signal bits.
  - Exceptions as channel messages.
  - Timer slack.
  - We have no multi-wait: a server waits on one ring, so events are multiplexed by hand.
- **Linux, for daily-driver behaviour:**
  - NO_HZ idle (a laptop that sleeps between events).
  - TSC-deadline timer.
  - x2APIC.
  - Per-CPU allocators (we have the lock-free buddy, but no per-core magazines).
  - RCU-style read paths.
  - Pressure-stall information.
  - We have none of the first two, which costs battery on the x17.
- **NOVA and Hedron:**
  - Portals with a per-call stack switch.
  - Delegation on IPC.
  - Our PKU gates (Phase 6) are the SASOS answer to portals, and are already cheaper (~54 TSC core).

Research still needed (each answered by a short note in the PR that does the work):

- **R1. Multi-wait.**
  - Should endpoints and notifications share one wait object (a Zircon-style port), or should each service thread bind one notification to its endpoint (seL4's bound notification)?
  - Recommendation: bound notification. It is smaller and needs no new kind.
  - To check: Display and Shell event loops need more than one endpoint plus one notification.
- **R2. TSC on the x17.**
  - CPUID 15h/16h on Tiger Lake gives the crystal ratio.
  - Check the invariant-TSC bit and whether the firmware leaves `IA32_TSC_ADJUST` consistent across cores.
- **R3. Jitter mode.**
  - What perturbation finds IPC/lifecycle races fastest?
  - Candidates: random preemption at kernel exit, random delay before `unpark`, random core for wakes.
  - Measure against today's three known races, reverted in a test build, before choosing.
- **R4. SMAP with PKU.**
  - The cost of `stac/clac` around user copies, plus a caller-keyed PKRU in the kernel, on Tiger Lake.
  - Done right, a kernel bug can only touch the caller's memory.

## 5. The plan

Seven streams, K1..K7. Each is one or more PRs, merged on green. Lines are against the
1,140 left in the budget; deletions are counted first.

| Stream | What | Kernel lines | Gate / target |
|---|---|---|---|
| **K1. One IPC** | (a) Fix v2's 1-core stall (#426's counters say where) and run `ipc2` in the KVM lane. (b) `sex_pdx::pdx_call`/`reply`/102/103 become v2 calls underneath: every Domain capability is an endpoint capability with the domain's default badge (BOOT wiring unchanged). (c) Files first: each deferred request keeps its own reply cap; `reply_later` queues go away. (d) Then Display, Shell, Bell, Net, the rest. (e) Delete `message_ring` for calls, `ReplyCredits`, `incoming_replies`, `ipc::router`/`sender`, 100/101/28/29, with an ABI note. | +250, **−900** | Same-core KVM round trip ≤1,500 cycles (no PKU), ≤2,000 (PKU); Files kernel entries per request ≤2; no `reply_later` in any service |
| **K2. Events, not polls** | (a) A **lifecycle notification** per holder: the kernel sets one bit per row on exit/rebirth and wakes once; holders read the pending set with one syscall op and ack with the run generation. (b) The six-party REBORN quorum becomes one per-holder state machine with generations; stale acks are refused by the kernel. (c) Tick housekeeping moves to a **per-core deferred-work list**: the event (task exit, restart due, IOMMU fault, ACPI GPE) queues the work, and the tick only runs what is queued. (d) Paging-exit watches and service restart move onto the same list. | +300, **−450** | Zero `[app.exit.notice]` resends in the Programs job; the tick does no work when nothing is queued (counter); restart-replay regression test (a late ack for run *n* is refused) |
| **K3. Scheduler for real workloads** | (a) Land #468 (cross-core handoff + preempt) and make every wake that answers a call use it. (b) **Scheduling contexts** (budget, period, priority) as a capability kind; passive servers run on the caller's donated context, so Files' time is charged to the program that asked. This generalises the #402 deadline class. (c) **Pair affinity**: a domain migrates when its core is above average load and its hottest IPC partner is elsewhere. (d) Per-core run state for program rows (views made it possible). | +350, −150 | 4-core latency gate **required** in CI, 10/10 passes, p99 ≤2 ms; KVM 4-core round trip ≤10k cycles (from ~68k); 8 programs on 4 cores ≥3x over 1 core |
| **K4. Laptop-grade time and idle** | (a) **TSC-deadline** one-shot timer and CPUID-15h/16h TSC calibration, with PIT and HPET as fallbacks only. (b) **Tickless idle**: a core with nothing queued (K2) and no timed waits programs the next deadline and enters the ACPI service's MWAIT C-state. (c) **x2APIC** (MSR access, faster IPIs, required past 255 APIC ids). (d) Timer wheel for timed waits. (e) IPIs batched per wake burst. | +300, −120 | Idle core wakeups ≤10/s (from 1,000); x17 boot log shows `tsc_deadline=1 x2apic=1`; no PIT access on a TSC-15h machine |
| **K5. Race hunting** | (a) **Jitter build** (`SEXOS_JITTER=seed`): random preemption at kernel exit, random delay before `unpark`, random core for wakes, with each perturbation logged with the seed. (b) Nightly: every app gate on the jitter build with 3 seeds; a failure files an issue with seed, gate and serial tail. (c) Today's three races, reverted in a test build, must be found within one nightly (R3). (d) KVM becomes the default lane for kernel gates; TCG becomes a nightly tripwire. | +120 (behind a cfg) | The three known races found by jitter; nightly green 7 days; median PR CI time down ≥30% |
| **K6. Proof over pins** | (a) Extend the #412 models to the scheduler's park/wake/handoff state machine (loom) and the K2 lifecycle state machine (Kani). (b) When a model covers a file's wait functions, drop that file's fn-v1 pin; pins remain only where no model exists. (c) A behaviour check in the boot proof replaces the source hash for the keyboard proof. | ±0 (tests outside `kernel/src`) | Kernel edits to modelled files need no pin; models run in CI under 5 min |
| **K7. Real-silicon hardening** | (a) **SMAP** with `copy_from_domain`/`copy_to_domain` under the caller's PKRU (audit S11, R4). (b) **IBPB** when leaving an untrusted domain (S14, branch s14 ready). (c) **INVPCID** for view and single-address flushes. (d) Mitigations chosen from the CPU's actual `ARCH_CAPABILITIES` and microcode, logged once. (e) **PKU gates for hot pairs**: Files→SexDrive block reads, Shell→Display surface commits (needs #465 and #466, then nested calls). | +250, −80 | `smap=1` on x17 and KVM; isolation matrix gate green with SMAP; gate call ≤150 cycles in the bench on KVM with PKU |

Net: about **+1,570 / −1,700**, so the kernel ends near 21,430 lines, below today.

### Order

1. **Now, in parallel:**
   - K1a: v2 stall and KVM lane, with the IPC v2 thread.
   - K3a: #468, with the cross-core IPC thread.
   - K5a and K5c: jitter build and race hunt, this thread.
   - K7b: IBPB, with the audit thread.
2. **Next:**
   - K2a/b (lifecycle notifications and generations), this thread. It removes today's storm and replay class directly.
   - K1b/c (Files onto v2).
3. **Then:**
   - K2c/d (deferred-work list) and K4 (time and idle). K4 needs K2c, because tickless idle is impossible while the tick polls.
   - K3b/c (scheduling contexts, affinity).
   - K1d/e (remaining services; delete the mailbox).
4. **Alongside:** K6 as each state machine settles, and K7a/c/e.

### What stays

These are the parts worth keeping as they are:
- BOOT_DAG as the only source of genesis.
- Restart-in-place.
- The crash log.
- IOMMU leases.
- Drag VMs.
- Views.
- PKU organs.
- One address space.

None of the streams above changes these rules, so this pass brings no decision that needs Andreas's sign-off beyond the earlier D1-D3. If K1e's deletions or K3b's scheduling contexts change a public ABI, the PR names the change, as before.

## 6. Owners

| Stream | Proposed owner |
|---|---|
| K1a, K1b-e | "Build IPC v2 endpoints" thread (owns `ipc_v2`, #426), with "Speed up microkernel IPC" (owns 102/103) |
| K2 | "Study and redesign the microkernel" (this thread) |
| K3a, K3c | "Speed up cross-core IPC" (#468) |
| K3b | This thread, after K2 |
| K4 | "Add device power management and suspend" for MWAIT/C-states; the timer and APIC parts in this thread |
| K5 | This thread, with "Fix open bugs and flaky CI" for the nightly job |
| K6 | The phase 7 models thread (#412) |
| K7a, K7b | "Rewrite kernel architecture and audit security" |
| K7c | "Add protection views with PCID" |
| K7e | This thread (PKU gates) |
