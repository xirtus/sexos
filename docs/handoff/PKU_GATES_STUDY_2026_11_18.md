# PKU gates: side-channel review and design (kernel redesign Phase 6)

Date: 2026-11-18. Status: study only, no kernel code. Decision D3 below is for Andreas.
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

1. **Gate code page.** The kernel generates it at boot and maps it user-executable and
   read-only. It holds one stub per *(callee, entry)* pair. Each stub uses a direct
   `call rel32` into the callee's entry, so the gate has **no indirect branches**. The
   page holds the only `WRPKRU` bytes outside the kernel, and the loader and BOOT
   scans exempt exactly this page.
2. **Call matrix.** The kernel writes it into a read-only page. Each row is
   `(caller key, callee, entry) → allowed, callee PKRU, callee stack slot`. It is built
   from D1 capabilities: a new `Gate` capability kind (callee + entry mask) is minted
   from the callee's BOOT root. Granting one sets a matrix bit; revoking it clears the
   bit. The matrix isn't secret, so any domain may read it.
3. **Gate data key.** One spare protection key covers the per-task gate frames
   (saved caller RSP, caller PKRU, return address, nesting depth) and the per-task
   callee stacks. Only the gate ever opens this key. The exact key is chosen in P6.1
   against the current key map.

### 4.2 A call

1. The caller does a direct `call` to the stub for *(callee, entry)*, with arguments in
   the IPC v2 message registers.
2. **Stub, step 1.** `RDPKRU` gives the caller's key set. That value is unforgeable,
   because only the gate and the kernel can write PKRU. The stub uses it to index the
   call matrix:
   - the index is bounds-checked and masked with `cmp` + `sbb`, the same as audit S10;
   - if the bit is clear, it returns `ERR_DENIED`.
3. **Step 2.** `WRPKRU` to (caller keys + gate data key). The new value comes from an
   immediate built from the matrix row, never from the caller's registers. A check
   follows the write (ERIM): `cmp eax, expected; jne fault`.
4. **Step 3.** Push a frame (caller RSP, caller PKRU, return RIP, depth) into the task's
   gate frame area, then load the callee stack pointer for this task.
5. **Step 4.** `WRPKRU` to the callee's PKRU, without the gate data key, then check it.
   Zero every register except the arguments: general-purpose registers, the direction
   flag, and vector state through `VZEROALL`, or a full XMM/ZMM wipe when the callee is
   marked as handling secrets. Then do a direct `call` to the callee entry.
6. **Return.** This mirrors the call:
   - `WRPKRU` to (callee + gate data), checked;
   - pop the frame, scrub non-result registers, then `WRPKRU` to the saved caller PKRU,
     checked against the frame;
   - restore the caller's RSP and `ret` to the saved return address.

That is four checked `WRPKRU` per round trip, which is about 2x the measured gate model:
around 100–120 ticks before scrubbing.

### 4.3 Why jumping into the middle doesn't help an attacker

- **Jumping to any `WRPKRU`.** The value written is an immediate or a value loaded from
  memory the attacker can't write, never an attacker register. The check after the
  write either confirms the expected value or faults with `ud2`.
- **Jumping past the matrix check, straight to step 3.** The attacker gets PKRU =
  (its own keys + gate data) and then falls into step 4. That writes the callee's PKRU
  and calls the callee's *legitimate entry*: exactly a normal call, minus the
  authorisation check.
  - To close this, step 3 re-derives the row from `RDPKRU`, which still shows only the
    caller's keys, and re-checks the matrix bit after opening the data key. The check
    and the use are then in the same straight-line block. This follows the ERIM and
    Hodor pattern: checks sit after the privileged write, not before it.
- **Jumping into the return path.** With an empty frame stack (depth 0), the stub
  faults. Frames live in gate-key memory, so neither side can forge one.

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

## 7. Prototype plan (if D3 is approved)

| Step | What | Kernel lines | Gate |
|---|---|---|---|
| P6.1 | Gate page generator (stubs, RSB fill, scrub), matrix page, gate data key, `Gate` capability kind; host tests of the generated bytes (no indirect branch, one `WRPKRU` set, checks after every write) | +180 | Host tests; the isolation matrix gets "jump into the stub middle faults" |
| P6.2 | Per-task gate frames and callee stacks; crash unwinding to `ERR_PEER_DIED`; accounting to the callee | +90 | Kill-the-callee-mid-call gate |
| P6.3 | One real pair behind a feature flag (Files → sexdrive block reads), with IPC v2 fallback | +30 | Files gates green both ways; bench: same-core service call ≤150 cycles KVM with PKU |
| P6.4 | Isolation matrix rows (Phase 7) for gates: no capability means no entry, revoke stops new calls, callee can't read the caller, caller's registers are scrubbed | 0 | Matrix green on KVM with PKU, skipped without PKU |

Total: about +300 kernel lines, matching the plan's Phase 6 estimate. The 22,700 budget is
not raised.

**Order.** The prototype starts after Phase 5b views land: the gate data key and the
key-8 rows both depend on the final key map. P6.1's generator and host tests can start
earlier, because they touch no shared kernel files.

## 8. Decision D3 for Andreas

Should SexOS add kernel-free calls between trusted services (PKU gates)?

- **A. Yes, prototype after views land (recommended).** Build P6.1–P6.4 behind a feature
  flag for one trusted pair, with IPC v2 as the fallback. Expected gain: service-to-service
  calls about 7–10x faster than the IPC v2 target.
- **B. Not yet.** Wait until IPC v2 meets its 1,000-cycle target, then decide whether the
  remaining gap matters.
- **C. No.** Keep every cross-domain call in the kernel. This is simplest, but it gives up
  the largest remaining IPC speedup for hot service pairs.
