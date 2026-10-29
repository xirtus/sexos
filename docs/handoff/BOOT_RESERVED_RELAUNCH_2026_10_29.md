# BOOT-reserved Quil relaunch — 29 October 2026

Historical checkpoint `33817053`. The [Spindle follow-up](BOOT_RESERVED_SPINDLE_RELAUNCH_2026_10_29.md) adds the second logical app reserve; the one-app limits below describe this checkpoint.

This follows launcher checkpoint `5c879302`. One short Astra read-only audit checked dormant delivery, hardware-key reuse and commit ordering; implementation and verification stayed in the parent.

## Delivered authority

BOOT reserves a second Quil protection domain and main task, both identity16, with native surface301. It loads a fresh copy of the relocatable image at layout16 (`0x4f000000`) and creates distinct user/kernel stacks and permanent control objects. Layout identity is separate from the hardware key: the successor uses key9 only after predecessor9 retires. All successor private leaves are cleared and locally invalidated before user execution starts.

The capability graph includes the successor's display/storage/shell edges, lifecycle slot45, close46, call47 and self-catalog QUERY48. Shell3 holds activation slot44, bound to logical Quil, incarnation2, predecessor9/task9 and successor16/task16. These are immutable genesis grants, sealed with the rest of BOOT_DAG. Services reserve four custody records before runtime. No runtime registry insertion, capability mint, frame selection, ELF load, or per-process page table was added.

Syscall66 observes or invokes that exact activation capability. Ready requires predecessor execution/stack custody release, all five independent service ACKs and completed private-leaf retirement. The successor must still be the exact Constructed task/trampoline, with matching context domain identity and every reserved leaf absent. All fallible checks precede restoring its BOOT-recorded leaves. The serialized BSP commit publishes the receipt and active routing before scheduler publication. Duplicate invocation returns the receipt without restoring pages or admitting another task. Ordinary wake and both scheduler BOOT admission paths cannot activate the dormant task.

Dormant targets reject capability calls, signals and replies. Predecessor target9, surface156/frame201, task9 and capability objects remain terminal tombstones. Successor backing is physically distinct from all other enrolled images; Sext continues to own and overwrite predecessor backing through its separate pool alias. The reserved successor pool alias remains Sext's BOOT authority, but cannot be retired or admitted as a workspace until that successor actually dies and satisfies its own cleanup barriers.

This is BSP-scoped. Activation refuses any history with more than one initialized execution core, as well as a caller executing away from core0. Local invalidation does not establish an SMP shootdown guarantee. AP entry remains parked; extending execution to other cores requires completed cross-core retirement first.

## Native desktop behavior

The launcher offers **Reopen / reserved instance** only when activation is ready. During predecessor cleanup it stays disabled. After activation it resolves the exact successor lifecycle entry; the app registers its fresh surface through its BOOT shell edge, receives normal frame/chrome/tiling and gets native presentation acknowledgement from the display.

Keyboard and document actions select the successor's pre-existing route; they do not mutate the predecessor route. The app reads its own immutable catalog instead of treating PD9/task9/surface156 as universal constants. Its legacy PD9 RamFS startup fixture is skipped for the successor because the old private object lease does not transfer. Normal document operations use the successor's own BOOT storage capability. Text status and geometry evidence records are atomic.

Closing the successor runs the same independent shell/display/files/net/Bell cleanup and private-frame retirement. The launcher then reports **Closed / reserve exhausted**. This slice provisions one successor for Quil; Spindle and Kaleidoscope remain reboot-only after Close. No identity is reset or reused, and there is no unbounded relaunch claim.

## Verification and evidence

The final sealed image passed its complete 90-second master window in `/tmp/sexos-relaunch-build-final/`, plus normal two-client paging recovery/reclamation. The host inventory passed **38 gates / 432 Rust test executions**, and the Python suite passed **232 tests**. All 327 bound source hashes were rechecked unchanged. Host substitutes separately cover wrong holders/tasks/slots, early activation, dormant routing, changed leaf rejection before any restore, second-core denial, duplicate receipts, permanent predecessor death and exhausted successor state. Shell production helpers are exercised with explicit register-transport doubles.

The reply-router concurrency test exposed an existing fixture race: `try_lock` at wake confused another sender's legitimate lock ownership with the waking sender retaining its own lock. The fixture now tracks lock custody per calling thread and checks published replies under the lock; production routing is unchanged.

Intermediate evidence remains in `/tmp`: the first native run could not establish startup geometry because a non-atomic tile record interleaved with timer output; the next run established actual activation and successor presentation but the harness selected the legacy row0 stub instead of Load row2. The corrected harness retains actual pixel checks, selects Load explicitly and takes the case-preserving status text from the native intent instead of assuming an uppercase document name. A 50-second master window also missed display clock reception; the completed 90-second image run passed clock reception and the two-client paging regression. These earlier incomplete attempts are not final acceptance evidence.

## Next bounded gains

1. Reserve a second logical app's distinct successor and extend the same physical/native acceptance, without assuming domain ID equals hardware key.
2. Give pending activation/registration explicit UI progress and cancellation semantics; preserve the immutable one-shot receipt if visual registration retries.
3. Design reserve replenishment as an explicit BOOT-sanctioned image/backing/authority protocol. Do not replenish by clearing terminal flags, reusing capability subjects or taking Sext journal frames.
4. Add cross-core invalidation completion before permitting activation beyond BSP execution.

## Final native acceptance

`/tmp/sexos-relaunch-desktop-verified/` passed the real QMP keyboard route, with no synthetic app activation:

- Saved document2 in predecessor9: **242 bytes**, fingerprint **`0xdbc6d3de7f43121f`**. After predecessor retirement, task16 loaded exactly the same bytes/fingerprint through its own storage edge.
- Original task9 stayed dead. Dormant delivery and early activation were rejected; stale predecessor delivery remained rejected after activation. Duplicate activation replayed its receipt, and exactly one successor commit/start was observed.
- The new catalog subject registered surface301 and received authenticated native presentation. A real Z key changed its document to **243 bytes, dirty**. The complete `n-v1 243B*` status glyphs matched the actual guest framebuffer using the production font; this was not a log-only visibility check.
- Close16 completed all five role ACKs, display commit, exact private retirement and Sext full-pool zero/reuse. Quil9, Quil16, Spindle12 and Kaleidoscope15 together retired **196 pages / 802816 bytes**. Both Quil tombstones remain permanent.
- Further Enter was rejected with `boot_reserve_exhausted`; actual launcher glyphs showed the exhausted state. Surviving Linen refresh/input overlap, filename pixels and F8 hide/restore with selection retention also passed.

[Successor after real input](assets/sexos-quil-successor-2026-10-29.png) · [Exhausted launcher](assets/sexos-reserve-exhausted-2026-10-29.png) · [Source/artifact evidence](assets/sexos-relaunch-2026-10-29-evidence.json).

Normal ISO SHA-256: `671a2685349bf0fd7a50b88cf6ec609d6433efa70ea62a99f145485c411b93e0`.
Kernel SHA-256: `cbf3be8abbec20af323dab7c2c8f3818cf0f7fedcd3c9974bb7a7f1c70a1ece8`.

The root normal ISO and kernel match the saved tested artifacts. `ARCHITECTURE.md` is unchanged. This round establishes same-boot cross-incarnation document recovery; the earlier checkpoint's separate reboot and USB runs are not claimed as fresh evidence for this image. No emulator remains running after acceptance.
