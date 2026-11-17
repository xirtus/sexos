# Kernel study and redesign plan (2026-11-17)

Requested by Andreas on 2026-11-17: study the SexOS microkernel and see whether we can make
it vastly faster, safer, more efficient and more capable. This document is the study and
the plan only. No kernel code changes with it; each phase below becomes its own PR after
Andreas has read this.

Basis: master at `2fb1fcd0` (kernel 20,238 non-blank lines, budget 22,700), plus the open
WS4 IPC PR #402 (`claude/project-thread-aql7vg`, +267 lines, syscalls 102/103/104).

## 1. Short answer

Yes. The kernel is sound in its big ideas (one address space, PKU organs, capabilities,
servers outside ring 0, a line budget) but it was built feature by feature, and the hot
paths carry a year of defensive scaffolding. The biggest wins are not in micro-tuning;
they come from five structural changes:

1. **IPC as a rendezvous, not a mailbox.** Today a call is "enqueue a message in the
   callee's 256-slot ring, take two spin locks, wake the callee through the general
   scheduler". seL4/L4-style synchronous IPC with a direct switch and a register
   fast path is 10x to 30x cheaper on the same hardware.
2. **Badged endpoints and one-shot reply capabilities.** The kernel would stamp each
   call with an unforgeable badge that dies with the caller's incarnation. That removes
   the per-request "is the caller still alive" checks Files does today (the last ~3
   kernel entries per IPC trip that #402 could not remove).
3. **Runtime capability delegation with revocation.** Capability tables are sealed after
   BOOT, so every dynamic right (file grants, program rows, Collar permissions) is
   re-implemented and re-checked inside servers. A small derivation scheme (mint from a
   BOOT-sanctioned root, revoke by generation) moves that to one kernel check.
4. **Protection views instead of key time-sharing.** There are 16 PKU keys. Program rows
   past the first few share key 8, take turns by flipping page-table bits and flushing the
   TLB, and all run on one core. Without PKU there is no memory isolation between
   services at all. Per-core, PCID-tagged permission views over the *same* translation
   fix both while keeping one address space.
5. **SMP by design.** Domains get a fixed home core from their id, shared-key programs all
   land on the last spare core, and the frame allocator, `GLOBAL_VAS`, the app table and
   the program-record table are global spin locks. The 4-core latency gate already fails
   on master.

Done incrementally (section 7), these keep every app and CI gate working at each step.
I do not recommend a clean-slate kernel: the authority model, BOOT_DAG, Drag VMs,
IOMMU leases, restart-in-place and crash log are worth keeping, and a rewrite would stall
all other work for weeks.

Realistic end state, measured on KVM with real PKU (section 3 explains why TCG numbers
cannot be the target): same-core IPC round trip under 1,000 cycles (seL4-class),
cross-core under 3 µs, the 4-core latency gate passing with p99 under 2 ms, AVX available
to programs, isolation that holds with or without PKU, and a kernel that ends the plan
*smaller* than today.

## 2. What the kernel is today

| Area | How it works now | Where |
|---|---|---|
| Entry | `SYSCALL`/`SYSRET`, per-task kernel stack, PKRU saved and set to 0 ("god mode") on every entry, restored on exit. Registers are pushed, then copied a second time into `SyscallRegs`. | `interrupts.rs` `syscall_entry` |
| Dispatch | One `match` over ~50 syscall numbers; selector validation first; per-syscall `cpu_account::count_syscall` atomic. | `syscalls/mod.rs` |
| IPC call | Look up a capability (copied out of a `Box`), check `accepts_ipc` (scan of the app target table), lock the callee's `reply_credits` (spin mutex, 128-entry probe table), enqueue a `MessageType` into its 256-entry ring, unpark its main task. Payload: opcode + 3 words. | `ipc.rs` `traverse_edge` |
| IPC reply | Check the replier holds a credit (lock), lock the caller's `incoming_replies` (`VecDeque`, capacity 1), push, consume credit, unpark. | `ipc/router.rs` `send_reply` |
| Fast path (#402) | 102/103 return the next event in registers; a per-core `handoff` slot runs the partner next; a short spin before halting. | `ipc/recv.rs`, `scheduler.rs` |
| Scheduler | 128 static per-core schedulers; work-stealing run queue + urgent queue + handoff slot + (#402) EDF deadline class. Every switch runs `tick()`: admission retry, steal, corruption heuristics on the picked task, latency notes, `cpu_account`, `program_records::before_dispatch` (takes the record lock), `program_tenancy::before_dispatch`, log budgets. Non-preemptible kernel. | `scheduler.rs` |
| Placement | `smp::home_core(pd)`: desktop services on core 0, apps spread by `pd % spare`, all shared-key program rows on the *last* spare core. No migration. | `smp.rs` |
| Capabilities | Per domain: 1,024 `AtomicPtr<Capability>` slots, each a heap `Box`. Filled at BOOT, then sealed; revocation only removes. No generations, no derivation, no badges. 25 kinds, several unused (`IPC` always invalid, `Socket`, `Network`, `Node`, `VfsNode`, `RemoteProxy`, `Spawn`, CHERI metadata always 0). | `capability.rs`, `capability_table.rs` |
| Isolation | PKU: keys 0 kernel, 1–4 core services, 8 shared program rows, 10 records, 14 shared, 15 untrusted. Loader refuses code containing WRPKRU/XRSTOR/XRSTORS bytes (ERIM-style). Shared rows take turns by toggling the user bit of 2 MiB PDEs + full local TLB flush. | `pku.rs`, `program_tenancy.rs`, `program_records.rs`, `sex_pdx::program_code` |
| Memory | One buddy allocator behind one spin lock; one `GLOBAL_VAS` spin lock; 64 GiB kernel heap window; kernel stacks are `Vec`s leaked with `mem::forget`; TLB shootdown by IPI (`smp::tlb`). No PCID. | `memory/*`, `smp.rs` |
| FPU | Eager `fxsave64`/`fxrstor64` on every switch. XCR0 is held to x87+SSE, so **AVX/AVX2/AVX-512 are off for everyone**. | `fpu.rs` |
| Size | 20,238 non-blank lines; 696 `unsafe`, 40 `static mut`, 387 `assert!/panic!/expect/unwrap`, 584 `serial_println!`, 55 mutexes. | |

Measured on QEMU TCG (from #402): IPC round trip 27k cycles (1 core, no PKU), 52k
(4 cores), 159k (PKU emulated); null syscall 2.3k / 0.6k / 65k. The 4-core latency gate
fails on master ("busy program never starts", p99 ~418 ms).

## 3. Findings, ranked

### F1. We are optimising against the wrong clock

TCG emulates every instruction; WRPKRU costs ~3k cycles there and a syscall tens of
thousands. Real hardware: WRPKRU is ~20–30 cycles, SYSCALL+SYSRET ~70–100. Ratios between
code paths are distorted, so a change that wins on TCG can lose on silicon and the PKU
column is mostly emulator overhead. The null-syscall figures above (2.3k on 1 core, 0.6k on
4) are themselves a TCG artefact.

GitHub runners have `/dev/kvm` (used for Drag). Running the bench under KVM with
`-cpu host` gives real cycle counts and real PKU on the runners' EPYC/Xeon CPUs. **Every
target in this plan is a KVM number**, with the TCG number kept only as a regression
tripwire.

### F2. IPC is a mailbox with a scheduler in the middle

A round trip today: two capability lookups, two `accepts_ipc` scans, four spin-lock
acquisitions (credits and replies, each side), two ring operations, two heap-backed
queues, two passes through the general `tick()` (with its admission retry, steal,
corruption checks, accounting and record-table lock), two eager FPU save/restores, and a
3-word payload that forces anything larger through shared regions. #402's handoff slot
and register returns cut the scheduler wait, but the structure is still asynchronous
enqueue + wake.

seL4, NOVA and Fiasco.OC all do the opposite: a call finds the server *already blocked
on the endpoint*, copies a few registers, switches stacks and address-space state
directly, and returns to user mode without touching a run queue. That is why their
one-way IPC is a few hundred cycles on modern x86.

### F3. Authority checks live in servers because the kernel cannot express them

Capabilities are sealed after BOOT and carry no generation or badge. The kernel stamps
`(caller_pd, caller_task)` on a message, but domain ids are reused across incarnations,
so Files must ask the kernel on every request whether that caller is still the same live
incarnation (app catalog + app info + its own crash-log epoch: the ~3 kernel entries
#402 left in place, correctly). Collar grants, file grants and program rows are each
their own server-side table.

seL4 solves this with **badged endpoint capabilities** (the server learns *which
capability* was used, unforgeably) and **one-shot reply capabilities**. When a caller
dies, its capabilities are revoked, so a stale badge can never arrive. The server does
one table lookup by badge, with no kernel entry.

### F4. Isolation has two holes and one ceiling

- **No PKU, no isolation.** Every domain's pages are user-accessible in one page table,
  and PKU is the only fence. The architecture says "PKU absent = same authority model,
  softer enforcement", but in practice it means any service can read or write any other
  service's memory. That rules out AMD before Zen 3 and any VM without PKU passthrough,
  and the "no PKU" benchmark numbers are for a mode with no memory protection.
- **Sixteen keys.** Programs past the first few share key 8 and take turns: open the next
  row's PDE user bits, close the last one's, flush the whole TLB, all on one core.
  That is safe but it serialises every such program onto one core and throws the TLB away
  on each turn.
- **Ring 0 sees everything.** `syscall_entry` sets PKRU to 0 for the whole syscall.
  Any kernel bug that writes through a user pointer can write any domain. SMAP/SMEP
  status and Spectre/Meltdown-PK mitigations are not visible in the kernel source and
  need an audit (section 6, Phase 1).

### F5. SMP is placement by formula plus global locks

`home_core` is a pure function of the domain id, with no load feedback and no migration.
All shared-key programs share the last spare core (by necessity, see F4). The frame
allocator, `GLOBAL_VAS`, `APPS`, the program-record table (taken inside `tick()`), the
tenancy pool and the reply-credit tables are spin locks. Cross-core wakes cost an IPI
each. The 4-core latency gate failure ("busy program never starts") is consistent with a
program row pinned to a core that is already busy and no way to move it.

### F6. The FPU policy costs speed and capability

Eager `fxsave`/`fxrstor` on every switch (~0.7k TCG cycles each side per #402's profile),
and no AVX at all. FFmpeg, Polyamour, Kaleidoscope (V8's JIT), Moan's DSP and Splicy's
encoders all lose 2x–4x on vector code without AVX2. XSAVE with AVX/AVX-512 can be
enabled while keeping PKRU *out* of XCR0, so the loader's XRSTOR ban still protects the
key register.

### F7. Scaffolding on the hot path, and a spec that drifted

- `tick()` contains corruption heuristics for a heap bug that was fixed long ago,
  `serial_println!` budgets read on every switch, and boot-phase `assert!`s per tick.
- Modules with no callers: `ucgm`, `sunni`, `protection`, `pdx`, `throughput_test`
  (commented out), likely `vga`; `cheri` is referenced only by a capability kind that
  carries a zeroed field. The `signal_handlers` map is a POSIX-shaped leftover.
- `ARCHITECTURE.md` describes a system that differs from the code: the determinism
  invariant (state is a pure function of epoch, input and graph) is not true with IPIs
  and timers; "the kernel does not map memory" is not true (`boot_memory`,
  `app_resources`, `program_records` map pages); the slot table is out of date. A spec
  that is not true cannot guide a redesign, so it gets rewritten in Phase 1.

### F8. Correctness checks worth running before redesign

- `SYSRET` with a non-canonical RCX faults in ring 0 on Intel (the CVE-2012-0217 class).
  Confirm the top user page below the canonical hole is never mapped executable.
- Kernel stacks are leaked `Vec`s; confirm task exit cannot exhaust them under the
  program-row churn the nightly soak runs.
- Capability `Box`es replaced by `insert`/`revoke` are never freed (comment says "RCU
  reclamation omitted"); harmless while tables are sealed, a leak once they are not.

## 4. How the reference kernels do it

| Kernel | IPC model | Authority | Scheduling | What we take |
|---|---|---|---|---|
| **seL4** | Synchronous endpoints, `Call`/`ReplyRecv`, register fastpath, notifications (bitmask) for async | Capability derivation tree, badges, one-shot reply caps, revocation | MCS: scheduling contexts (budget/period) donated across IPC; passive servers | Fastpath shape, badges, reply caps, notifications, scheduling-context donation, proofs as a design discipline (small, total functions) |
| **NOVA** | Portals (one per entry point), synchronous, execution contexts | Per-domain capability spaces, delegation via IPC "typed items" | Scheduling contexts donated through portals | Portals = one entry per service method; delegation carried in the message |
| **Fiasco.OC / L4Re** | Synchronous IPC gates, IRQ objects | Object capabilities with delegation and unmap | Fixed-priority, per-core, explicit migration | IRQ objects bound to a gate; per-core kernel with explicit cross-core paths |
| **Zircon** | Channels (async, messages + handles), ports for waits | Handles with rights, transferred in messages; jobs/policies | Fair scheduler, per-CPU, profile-based deadlines | Handle *rights* bits, handle transfer in messages, ports as the single wait primitive |
| **Composite** | Thread migration across components, no message copy | Capability-based invocation | User-level schedulers | Migrating calls for the PKU gate idea (Phase 6) |
| **ERIM / Hodor / Donky** (PKU research) | Same-thread domain switch via WRPKRU in a vetted gate, ~tens of cycles | Binary inspection bans stray WRPKRU/XRSTOR | n/a | We already have the binary inspection; a kernel-owned gate page makes trusted-service calls cost no kernel entry |

None of these is a single address space system with PKU organs; SexOS's combination is
unusual, and its advantage is exactly that a domain switch does not need a page-table
switch. The plan keeps that and adopts the IPC/authority/scheduling mechanisms that are
proven elsewhere.

## 5. Target design ("kernel v2", built inside the current kernel)

### 5.1 IPC

- **Endpoint** capability kind: a rendezvous object a server receives on. Each endpoint
  capability carries a 64-bit **badge** minted when the capability is granted; the
  kernel delivers the badge with every message. Domain capabilities become endpoint
  capabilities with the domain's default badge, so BOOT wiring keeps working.
- **Call / ReplyRecv** as the only synchronous pair (syscalls 102/103 keep their numbers
  and semantics; 100/101/28/29 become shims over them for one release, then go).
- **Fastpath**: if the server is blocked on the endpoint, same core, no pending
  higher-priority work: copy message registers, swap current task, set PKRU, `SYSRET`.
  No run queue, no `tick()`, no locks (the endpoint's waiter is a single atomic).
- **Message registers**: 8 words in registers (rdi, rsi, rdx, r10, r8, r9, r12, r13) plus
  a 4 KiB per-thread IPC buffer (already in the caller's own region, so no copy for the
  SASOS case; the kernel only validates the length and passes the pointer's
  region capability).
- **Reply capability**: one-shot, held in the server's thread state, consumed by the
  reply. Replaces `ReplyCredits` (128-entry probe tables, a lock per call) and
  `incoming_replies` (a `VecDeque` under a lock).
- **Notifications**: a 64-bit word per notification object, OR-ed by signalers, waited
  on by one thread. Replaces `signal_ring`, the `MessageType` enum's display/input
  variants, and the `InputRing` special case. Device IRQs bind to notifications.
- **Cross-core**: a call to a server homed elsewhere enqueues on that core's per-core
  inbox and IPIs once per batch; Phase 4 adds migration so hot pairs end up together.

### 5.2 Authority

- **BOOT_DAG still decides what may exist.** Each kind of runtime right gets a BOOT-held
  *root* capability (for example Files holds `FileGrantRoot`). Holding the root lets a
  server **mint** derived capabilities with fewer rights and a badge, and grant them to
  a domain that holds a receive slot for them. Nothing can be minted without a root, so
  the "no capability without BOOT sanction" rule still holds; what changes is that the
  graph is no longer frozen after BOOT. **This needs Andreas's OK** (decision D1).
- **Generations instead of a derivation tree.** Each slot holds `(object, generation,
  rights, badge)`. Revoking a root bumps its object's generation; every derived
  capability is checked against it on use (one compare). Cheaper and smaller than seL4's
  CDT, and enough for our "revoke everything an incarnation held" needs.
- **Incarnation death revokes.** A domain's exit bumps the generation of its endpoints
  and of every capability it minted, so no *new* call can arrive from a dead caller.
- **In-flight requests are cancelled too.** A badge already delivered survives revocation,
  so the badge carries the caller's incarnation generation, and the caller's death
  invalidates every outstanding one-shot reply capability for it (as seL4 cancels a
  reply object when its caller goes away). A server with deferred work (Files'
  restricted I/O) checks before completing that its reply capability is still live: a
  read of a kernel-published, read-only generation word, not a syscall. With both
  pieces, servers need no liveness syscalls.
- **Slots stored inline** (32 bytes each, no `Box`), with rights bits on every
  capability (read, write, grant, call, receive) checked in one place.

### 5.3 Isolation and memory

- **Protection views** (decision D2). One global translation (VA → PA identical
  everywhere: still a single address space), but up to N per-core *permission views*:
  page-table hierarchies with identical leaf translations. Permission bits live in the
  entry that points at a subtree, so each view has **private copies of every table level
  that holds a differing bit** (PML4, and the PDPT/PD pages above any 1 GiB/2 MiB span
  whose user bit differs between views); everything below those entries (the PT pages,
  and whole subtrees whose permissions are the same in all views) is shared. A private
  PD page is 4 KiB per 1 GiB of differing range, so the cost stays small. Each view is
  tagged with a PCID, so switching views is a
  CR3 write with no flush. Uses:
  - Shared-key program rows each get their own view: no more turn-taking, no full TLB
    flush, and they can run on any core at the same time.
  - Without PKU, every organ gets a view, so isolation holds on any x86-64 CPU (slower
    switch, still correct).
  - With PKU, organs that have a key keep using PKRU only (the fast path is unchanged).
- **PCID** for the remaining flush sites (the deferred #402 item) comes with views.
- **Per-core frame caches** (magazines) in front of the buddy allocator; `GLOBAL_VAS`
  split into per-region locks; kernel stacks from a slab with reuse.
- **Kernel access to user memory** goes through `copy_from_domain`/`copy_to_domain`
  helpers that set PKRU to *the caller's* keys (plus kernel) instead of 0, with SMAP on,
  so a kernel bug can at worst touch the caller's own memory.

### 5.4 Scheduling and SMP

- **Scheduling contexts** (budget, period, priority) as a capability, generalising the
  #402 deadline class. A server without its own context runs on the caller's donated
  one ("passive server"), so Files' time is charged to the program that asked.
- **Per-core everything**: run queues already are; make the record/tenancy state per
  core (views remove the reason it is global) and move accounting out of `tick()`.
- **Placement with feedback**: start from `home_core`, then migrate a domain when its
  core's load is above average and an IPC partner is elsewhere; keep hot call pairs on
  one core.
- **Tickless idle** and a timer wheel for timed waits.

### 5.5 FPU and vector state

- Enable XSAVE with x87, SSE, AVX, (AVX-512 where present); **never** the PKRU component.
- `XSAVEOPT`/`XSAVEC` with the init/modified optimisation, so a task that never touched
  vector registers costs almost nothing to switch. Kernel-to-kernel and IPC fastpath
  switches between tasks that have not used the FPU skip it entirely.

## 6. Phased plan

Each phase is one or more PRs, merged only on green (2026-11-16 merge rule). Each states
its kernel growth against the 22,700 budget. Targets are KVM `-cpu host` on the GitHub
runners unless marked TCG. "Today" columns come from Phase 0, so the KVM baselines are
filled in by that PR.

| Phase | What | Kernel lines | Gate / target |
|---|---|---|---|
| **0. Real numbers** | KVM bench lane in CI (`-cpu host`, PKU on and off), cycle counters for entry, dispatch, switch, IPC fast/slow path; publish a baseline table. Keep TCG as a tripwire. | +40 (counters behind a feature) | Baseline published; TCG numbers within 10% of #402 |
| **1. Clean and audit** | Remove dead modules and kinds (F7), take corruption heuristics and log budgets out of `tick()`, single-copy `SyscallRegs`, rewrite `ARCHITECTURE.md` to match reality, audit SYSRET/SMAP/SMEP/Spectre (F8), free capability boxes. | **−800 to −1,200** | All gates green; null syscall ≥20% faster on KVM; context switch ≥25% faster |
| **2. IPC v2** | Endpoints + badges, Call/ReplyRecv fastpath, 8 message registers + IPC buffer, one-shot reply caps, notifications; 100/101/28/29 as shims; Files and sex-app move first. | +600, −400 (credits, replies, rings) | Same-core round trip ≤1,000 cycles KVM no-PKU, ≤1,300 with PKU; TCG ipc_roundtrip ≤9k (the 3x #402 aimed for); Files kernel entries per request 5 → 2 |
| **3. Authority v2** (needs D1) | Roots, minting, generations, rights bits, inline slots; incarnation death revokes; Files/Collar/program rows move their grant tables onto it; Files drops its liveness syscalls. | +400, −300 | Zero liveness syscalls per Files request; revoke-all of a dead incarnation ≤10 µs; capability fuzzing (WS3 harness) clean for 24 h |
| **4. SMP** | Per-core frame magazines, split VAS lock, per-core record state, migration with IPC-pair affinity, batched cross-core IPIs, tickless idle, scheduling contexts with donation (generalising 104). | +500, −200 | 4-core latency gate passes 10/10 runs, p99 ≤2 ms; cross-core round trip ≤3 µs; 4-core file_stat within 1.2x of 1-core |
| **5. Views + PCID + XSAVE** (needs D2) | Protection views, PCID, program rows on any core, isolation without PKU, XSAVE with AVX/AVX-512 and init optimisation. | +500, −350 (tenancy turn-taking) | No full TLB flush on row switch; 8 programs on 4 cores scale ≥3x over 1 core; a no-PKU isolation gate (domain A cannot read B) passes; AVX2 FFmpeg decode ≥2x faster |
| **6. PKU gates** (research, decision D3 later) | Kernel-owned gate page holding the only WRPKRU; migrating calls between trusted services with no kernel entry; stack switch in the gate. | +300 | Same-core service call ≤150 cycles KVM with PKU; side-channel review written first |
| **7. Assurance** | Kani/proptest models of the capability table, endpoint state machine and scheduler queues; `loom` for the lock-free parts; syscall fuzzing in nightly; `unsafe` count down by half with each remaining block commented. | ±0 (tests live outside `kernel/src`) | Models in CI; nightly fuzz green 7 days |

Net kernel size after Phase 5: roughly 20,200 lines (around today's size) with
far more capability. The 22,700 budget is not raised.

Order and parallelism: 0 and 1 first (a week). 2 and 4 can overlap once 1 lands (they
touch different files). 3 needs 2's endpoints. 5 needs 4's per-core record state. 6
waits for 5. 7 runs alongside from Phase 2.

## 7. Keeping everything working

- **Shims, not flag days.** Old syscalls keep working as thin wrappers over the new
  paths for one release; `sex_pdx` and `sex-app` switch first, so every app moves at once
  by relinking. Shims are removed in the phase after, with an ABI note.
- **One service at a time.** Files moves to endpoints first (it is the IPC hot spot and
  has the bench), then Display/Shell, then the rest by usage.
- **Same gates.** Every existing CI gate stays; each phase adds its own gate from the
  table above. Long runs on GitHub CI from the pushed branch (2026-11-15 rule).
- **Guardrails hold.** No Linux-shaped APIs: endpoints, notifications and scheduling
  contexts are seL4-shaped mechanisms, and policy (who gets which root, placement hints)
  stays in servers.

## 8. Decisions for Andreas

**2026-11-17: Andreas approved D1 and D2.** D3 stays open until after Phase 5. D4 is done in Phase 1.

| | Question | Recommendation |
|---|---|---|
| **D1** | Allow capabilities to be minted after BOOT, from BOOT-held roots only, with generation-based revocation? This changes "the capability graph is immutable once the epoch engine starts" to "the *kinds and roots* are fixed at BOOT; derived capabilities come and go". | Yes. It is what lets servers stop re-checking authority, and it keeps BOOT_DAG as the only source of genesis. |
| **D2** | Allow per-core *permission views* (shared translation, separate root tables, PCID-tagged)? This touches "NO per-process page tables": translation stays global, but more than one root table exists. | Yes. It removes the 16-key ceiling and the one-core bottleneck, and gives isolation on CPUs without PKU. |
| **D3** | Later: allow user-mode PKU gates between trusted services (no kernel entry)? | Decide after Phase 5 with a written side-channel review. |
| **D4** | Rewrite `ARCHITECTURE.md` to describe the system as it is (drop the determinism invariant as a hard rule, or scope it to UiIR)? | Yes, in Phase 1. |

## 9. Coordination

- **#402 (WS4, "Speed up microkernel IPC")** lands first as is. Phase 2 builds on its
  102/103 numbers and handoff slot rather than replacing them, and the deferred PCID work
  moves into Phase 5.
- **Week plan file ownership** (WS1 Denim/pagefault/app_resources, WS2 memory/slab, WS4
  scheduler/IPC) stays in force this week. Phases 1+ start after the week's PRs merge,
  or take files only with the owning thread's agreement.
- **Drag (`vmx.rs`)** is untouched until Phase 5, where vCPU runs become an ordinary
  scheduling-context user.
