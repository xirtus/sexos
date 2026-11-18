# PKU gates: side-channel review and design (kernel redesign Phase 6)

Date: 2026-11-18. Status: study and design; D3 approved 2026-11-18 (prototype now).
Parent plan: `docs/handoff/KERNEL_STUDY_AND_REDESIGN_2026_11_17.md` (Phase 6, decision D3).

## 1. Short answer

A PKU gate lets one trusted service call another **without entering the kernel**. A
small, kernel-built code page holds the only `WRPKRU` instructions outside the kernel.
It switches PKRU to the callee's key, switches stacks and calls a fixed entry point.
On return it does the same in reverse.

- **Speed.** On a real CPU, the core of a gate (two checked `WRPKRU`, a stack switch and
  a call) measures about **50–60 TSC ticks**. The full design below needs about four
  `WRPKRU` plus register scrubbing, so roughly 100–150 ticks per round trip. For
  comparison, today's same-core IPC round trip is about 10,300 cycles (§5.6 of the
  plan), and Phase 2's target is 1,000. A gate would be about 7–10x faster than even
  the IPC v2 target.
- **Safety.** The gate is only as safe as two things that SexOS already has or is
  landing:
  - no stray `WRPKRU`/`XRSTOR` bytes anywhere else: the loader scan for programs, plus
    the BOOT-service scan (audit S15, PR #409);
  - PKRU kept out of XSAVE (Phase 5a, #408).
- **Not for untrusted programs.** Same-privilege domains in one address space share
  branch predictors, and user mode cannot flush them. A gate also keeps the caller's
  core and timing visible to the callee. So gates are for **trusted BOOT service pairs**
  (for example Files → sexdrive, Silk → sexdisplay). Programs keep using IPC v2
  system calls, where the kernel can add barriers.
- **Recommendation (D3).** Approve the design, and build the prototype behind a feature
  flag for one service pair, after protection views (Phase 5b) land. On a CPU without
  PKU, the same call stub falls back to an IPC v2 call.

## 2. Measurements

`tools/pku-gate-bench/pku_gate_bench.c` is a Linux userspace microbenchmark. It uses
`pkey_alloc` and `pkey_mprotect` on the host and measures each operation as the median of
15 batches. CI runs it on every push in the `boot` job, in the step "PKU gate cost on the
runner's CPU", on the same runner type as the KVM bench (Xeon 6975P-C, PKU present).

First local run (cloud container, Intel Xeon @ 2.30 GHz, PKU and OSPKE present). The
numbers are TSC ticks per operation, with the empty-loop cost subtracted:

| Operation | Ticks |
|---|---|
| `RDPKRU` | 16 |
| `WRPKRU`, same value | 52 |
| `WRPKRU` open + close (two writes) | 45 |
| Same, ERIM-checked (`cmp` + `ud2` after each write) | 45 |
| Stack switch + direct call only | 3 |
| **Gate model**: checked open, stack switch, call, switch back, checked close | **54** |
| Open, touch a protected page (read + write), close | 51 |
| Linux `getppid` system call (for scale) | 192 |

**The real stub (P6.1).** `crates/sex-gate/tests/linux_run.rs` runs generated stubs on the
host under real `pkey_alloc` keys. A full round trip costs **173 ticks** on the same
CPU, against 192 for `getppid`. That includes four checked `WRPKRU`, the frame
bookkeeping, two `VZEROALL`, the register scrub and a 16-entry RSB fill. The obvious
next cuts are scrubbing only the vector registers the callee policy needs, and a shorter
RSB fill on parts with eIBRS. Both are measured before the ≤150-cycle KVM gate in P6.3.
A release build of the same test measures 162 ticks, or 152 with a one-entry RSB fill.
Dropping `VZEROALL` makes the scrub test fail (the callee sees the caller's `xmm7`), so
the vector scrub stays.

**Reading.** A `WRPKRU` pair costs about the same as one serialising write; the check after
each write is free. The gate model costs about a quarter of a bare Linux system call.
The CI runner's numbers will be added here once the first run reports them.

## 3. Threat model

- **Callers and callees** are BOOT services, each with its own protection key and at
  least one task. Shared-key programs (key 8 rows) can't be gate callers. PKRU can't
  tell those programs apart, so the gate couldn't tell who is calling.
- **The attacker** is a compromised caller or callee. It has arbitrary code execution in
  its own domain at ring 3. It can jump anywhere, including into the middle of the gate
  page. It can set any register, and it can race other tasks on other cores.
- **Protected assets** are:
  - every other domain's memory;
  - the gate's own data (the call matrix and the per-task frames);
  - the integrity of returns: a callee can't return into the caller at a point the
    caller didn't choose, and it can't keep the caller's PKRU open;
  - revocation: a revoked gate capability can't be used again once the kernel
    acknowledges the revoke.
- **Out of scope**:
  - physical attacks;
  - bugs in the kernel itself (gates don't change the kernel's trusted computing base,
    except for the generated gate page);
  - denial of service by a callee that never returns. The caller's task is stuck, the
    same as a blocked IPC call; the kernel can still kill the callee, see §4.6.

## 4. Design

### 4.1 Pieces

1. **Gate code page.** The kernel generates this page at boot and maps it
   user-executable and read-only. It holds one stub per *(caller, callee, entry)* that
   the capabilities allow. Each stub enters the callee with a direct `call rel32`, so the
   gate has **no indirect branches**. In a single address space the kernel places the
   page within ±2 GiB of BOOT service text. The page holds the only `WRPKRU` bytes
   outside the kernel. The loader and BOOT scans exempt exactly this page, by address
   and hash.
2. **Call matrix.** The kernel writes this into a read-only page: one bit per stub. It
   is built from D1 capabilities. A new `Gate` capability kind (callee + entry mask) is
   minted from the callee's BOOT root. Granting one sets the bit, and revoking it clears
   the bit. The matrix isn't secret.
3. **Per-task gate area, found through the user GS base.** The kernel sets each task's
   user GS base to that task's gate area on every switch. `CR4.FSGSBASE` stays off, so
   ring 3 can't change it, and that makes "which task am I" unforgeable in user mode.
   Today the kernel sets GS_BASE and KERNEL_GS_BASE to the same per-core block, so
   P6.2 must give every entry path strict `swapgs` discipline. That is also worth doing
   as hardening on its own. The area has two parts:
   - a **frame page** under a spare *gate key* that only the gate opens. It holds the
     task's current domain, the frame stack (stub id and depth) and the callee stack
     pointers;
   - one **return-slot page per domain the task enters**, tagged with *that domain's*
     key. It holds the domain's pending caller RSP and stub id. The kernel allocates it
     the first time the task enters that domain.

### 4.2 The rule that makes jumping into the middle harmless

Any domain can jump to any byte of the gate page with any registers. So the danger is
not the authorisation check before a `WRPKRU`: it is what the code *after* the write
does with registers an attacker chose. ERIM's checked write (`wrpkru; cmp eax, IMM; jne
fault`) only proves that PKRU now equals `IMM`. A jumper who sets `eax = IMM` passes that
check. So every `WRPKRU` in the gate obeys three rules:

- it writes an immediate and is checked against that immediate;
- the immediate is one of three values only:
  - **G**: the gate key alone, every other key disabled. This is harmless: a jumper
    gets no one's memory and runs a fixed path;
  - **E**: the callee's keys, immediately followed by a direct `call` to the callee's
    fixed entry. A jumper gets nothing it couldn't get by calling E normally;
  - **C**: the caller's keys, followed only by code that takes its stack and return
    address **from C's own return slot** (C-key memory, unforgeable by anyone else);
- no register value the attacker chose is used as an address while PKRU is G or C.

The first draft of this design restored the caller's RSP from a register and then
returned with `ret`. A jumper landing on that last `WRPKRU` with `eax = C` would have
run code of its choice with C's keys. The rule above closes that hole.

### 4.3 A call and its return

Call (stub for C → E, entry n):

1. **Still in C's keys, no write yet.** Check the matrix bit; if it is clear, return
   `ERR_DENIED`. Store RSP and the stub id in the task's C return slot. A jumper from
   another domain faults right here, because it can't write C-key memory.
2. **Write G, checked.** Check that the task's current domain (from the frame page) is
   C; otherwise unwind to the kernel. Check the depth bound. Push the frame (stub id,
   depth), set the current domain to E, and load this task's E stack pointer.
3. **Scrub.** Zero every register except the four argument registers, clear the
   direction flag, and run `VZEROALL` (or a full XMM/ZMM wipe for callees marked as
   holding secrets).
4. **Write E, checked.** `call` the entry (direct).

Return (falls through after the `call`):

5. **Write G, checked.** Check that the top frame's stub id is this stub's id; pop it
   and set the current domain back to C.
6. **Scrub** non-result registers and **fill the RSB** with a fixed call sequence
   (SC4).
7. **Write C, checked.** Load RSP from the C return slot, check that its stub id
   matches, clear the slot, and `ret` to C's genuine call site.

That is four checked `WRPKRU` per round trip, about 2x the measured gate model: roughly
100–120 ticks before scrubbing and the RSB fill.

**Residual R1 (found late, not prevented).** Take a chain C → E → X inside one task.
X could jump to step 7 of the C → E stub and resume C at its genuine pending call site
with a result E never produced. Step 7 only accepts C's pending call *to this E*, so the
damage is a skipped E frame. The orphaned frames are detected at the task's next gate
entry, where step 2's current-domain check fails and the kernel unwinds. Callees that
call onward should treat their own callees' results with the same care as any input.

### 4.4 Interrupts, preemption and other cores

- Kernel entry already saves the user PKRU on the kernel stack and restores it on exit
  (`interrupts.rs`). So a task preempted mid-gate resumes with exactly the PKRU it had.
- Two tasks of one caller can call the same callee at once. Each task has its own
  callee stack slot and frame area, so nothing in the gate is shared between tasks.
  That means no locks and no cross-core state.
- PKRU is per hardware thread. Opening the gate key on one core never opens it on
  another.
- SexOS has no POSIX signals, so the ERIM and Hodor signal-delivery problems don't
  arise. A kernel upcall (if one is ever added) must run with the task's in-gate PKRU,
  never 0.

### 4.5 Accounting and faults

- While a task is inside a callee, the kernel attributes faults and CPU time to the
  **callee**. It identifies the callee from the task's PKRU, cross-checked with the
  gate frame. This is the same "passive server, caller's donated time" model as the
  Phase 4 scheduling contexts.
- A page fault inside the callee is the callee's fault, so the crash log names the
  callee.

### 4.6 Crashes and revocation

- **The callee dies mid-call.** The kernel walks the task's gate frames, pops back to
  the caller's frame, and resumes the caller at the return stub with
  `ERR_PEER_DIED`. This is the gate equivalent of IPC v2's cancelled reply capability.
- **The caller dies.** Its tasks are killed as usual. Their callee stack slots are
  freed when the task's gate area is freed.
- **Revocation.** The kernel clears the matrix bit, so new calls fail at once. Calls
  already in flight finish; they are bounded the same way an IPC call is. A
  strict-revoke option waits on other cores with an IPI until no frame for that row is
  live.

### 4.7 Fallback without PKU

On a CPU without PKU, the gate page is generated as plain IPC v2 call stubs (`syscall`
105). Services link against one stub ABI and don't know which one they got. Protection
views (Phase 5b) give the isolation, and IPC v2 gives the speed floor.

## 5. Side-channel review

The plan asks for this review before any gate code. Each item gives the exposure and
the mitigation.

| # | Channel | Exposure through a gate | Mitigation |
|---|---|---|---|
| SC1 | **Spectre v1** (bounds-check bypass) on the matrix index | A caller could make the stub speculatively read a matrix row out of bounds and leak it through the cache | The matrix is public, so a leak shows nothing secret. The index is still masked (`cmp` + `sbb`, as in S10) so a speculative out-of-bounds read can't pick a stack slot either |
| SC2 | **Spectre v2** (branch target injection) on gate branches | None in the gate itself: every branch is direct (`call rel32`, `jne`) | Generated stubs, no indirect branches; CI checks the generated page's bytes |
| SC3 | Spectre v2 on the **callee's own indirect branches**, trained by the caller | **Real, and not solvable in user mode.** Caller and callee run at the same privilege level in one address space, so eIBRS/IBRS don't separate them, and IBPB needs a privileged MSR write | Gates only between **mutually trusted** BOOT services. A callee that holds secrets (TLS keys in sexnet, the keyring) is never a gate callee for anything that handles untrusted input; it stays on IPC v2, where the kernel can IBPB on domain switch. This is why programs never use gates |
| SC4 | **Return-stack (RSB) poisoning** | The callee could leave poisoned RSB entries that the caller's next returns consume speculatively | The return path stuffs the RSB with a short fixed sequence of calls before returning to the caller (the standard RSB fill). Its cost is measured in P6.1 |
| SC5 | **Meltdown-PK** (transient read through a PKU-disabled key on affected Intel parts) | Same exposure as today's PKU isolation: a gate doesn't widen it, but it makes domain changes more frequent | Already in the stream 2 security audit (Meltdown-PK row). On affected parts, the kernel marks PKU isolation as soft, and gates are refused for any callee holding secrets |
| SC6 | **Register and vector state leakage** across the call | The callee would see the caller's leftover registers, and the reverse | Scrubbing in steps 4 and 6: all non-argument GPRs, the flags, and `VZEROALL`, or a full XMM/ZMM wipe for secret-handling callees. XCR0 excludes PKRU (Phase 5a), so `XRSTOR` can't load PKRU |
| SC7 | **Microarchitectural buffers** (MDS/TAA-class, same core) | Same-core, same-privilege sharing exists today between any two domains time-sharing a core | No user-mode `VERW` boundary is possible. This is the same exposure as two services on one core today. It is documented and scoped by the same trusted-pair rule as SC3 |
| SC8 | **Timing and contention** | The callee sees when and how often the caller calls; the caller sees the callee's run time | Inherent to synchronous calls, and true of IPC too. Not a goal |
| SC9 | **Rogue `WRPKRU`/`XRSTOR`** outside the gate page | Would let any domain open any key | Program scan at load and at JIT seal; the BOOT-service scan (S15, #409); generated gate page exempted by exact address and hash; PKRU never in XCR0 |
| SC10 | **Kernel restoring PKRU from memory the user controls** | Would let a domain set its own PKRU by forging a saved frame | The kernel restores user PKRU from its own stack or PD record. The P6.1 prototype adds a host check that no restore path reads user memory |

**Verdict.** Gates are sound for **trusted-to-trusted** calls once SC4 and SC6 are in the
stub and SC9 is enforced, which it is after #409. SC3 and SC7 can't be fixed in user mode.
They set the rule that **untrusted code never shares a gate with a secret-holding
service**.

## 6. Prior work this follows

- **ERIM** (USENIX Security 2019): binary inspection for `WRPKRU`/`XRSTOR` and the checked
  write pattern. SexOS already does the inspection part in its loader.
- **Hodor** (USENIX ATC 2019): debug-register watchpoints on stray `WRPKRU` pages instead
  of rejecting binaries. That isn't needed here, because SexOS controls every binary's
  layout and can refuse it.
- **libmpk** (USENIX ATC 2019): key virtualisation for more than 16 domains. SexOS's
  answer is protection views (Phase 5b) rather than key virtualisation.
- **Donky** (USENIX Security 2020): hardware support for a cheaper domain switch. This
  is a useful comparison for the measured `WRPKRU` cost.
- **seL4, NOVA, Fiasco.OC:** their fastpath IPC is the kernel-mediated alternative. Phase
  2 builds it, and IPC v2 is the fallback in §4.7.

## 7. Prototype plan (D3 approved)

| Step | What | Kernel lines | Gate |
|---|---|---|---|
| P6.1 (done 2026-11-18) | Gate stub generator as a crate (`crates/sex-gate`, outside `kernel/src`): stubs, RSB fill, scrub, matrix and area layout; host tests of the generated bytes (no indirect branch, every `WRPKRU` writes G/E/C and is checked, no stray key bytes from immediates) and a Linux run of real stubs under `pkey_alloc` keys (call, return, deny, jump-to-each-`WRPKRU`) | 0 | Host tests in CI |
| P6.2a | Kernel: per-task user GS base with strict `swapgs` on every entry path; gate area pages; matrix page; `Gate` capability kind | +150 | All boot gates green; GS base unforgeable from ring 3 |
| P6.2 | Per-task gate frames and callee stacks; crash unwinding to `ERR_PEER_DIED`; accounting to the callee | +90 | Kill-the-callee-mid-call gate |
| P6.3 | One real pair behind a feature flag (Files → sexdrive block reads), with IPC v2 fallback | +30 | Files gates green both ways; bench: same-core service call ≤150 cycles KVM with PKU |
| P6.4 | Isolation matrix rows (Phase 7) for gates: no capability means no entry, revoke stops new calls, callee can't read the caller, caller's registers are scrubbed | 0 | Matrix green on KVM with PKU, skipped without PKU |

Total: about +300 kernel lines, matching the plan's Phase 6 estimate. The 22,700 budget is
not raised.

**Order (Andreas chose "prototype now", 2026-11-18).** P6.1 starts at once; it touches no kernel files. The kernel steps start after Phase 5b views land: the gate data key and the
key-8 rows both depend on the final key map. P6.1's generator and host tests can start
earlier, because they touch no shared kernel files.

## 8. Decision D3 for Andreas

**2026-11-18: Andreas chose A (prototype now).**

Should SexOS add kernel-free calls between trusted services (PKU gates)?

- **A. Yes, prototype after views land (recommended).** Build P6.1–P6.4 behind a feature
  flag for one trusted pair, with IPC v2 as the fallback. Expected gain: service-to-service
  calls about 7–10x faster than the IPC v2 target.
- **B. Not yet.** Wait until IPC v2 meets its 1,000-cycle target, then decide whether the
  remaining gap matters.
- **C. No.** Keep every cross-domain call in the kernel. This is simplest, but it gives up
  the largest remaining IPC speedup for hot service pairs.
