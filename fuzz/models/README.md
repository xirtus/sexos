# Kernel models (kernel plan phase 7)

Host-only checks of kernel data structures. The tests live at the repo root as
`tests/model_*.rs` and build the **real** kernel sources through `#[path]`; nothing
here is linked into the kernel and no kernel line changes.

| Structure | proptest model | loom | Kani |
|---|---|---|---|
| `kernel/src/ipc/reply_credit.rs` | `model_reply_credit.rs` (vs a map, incl. full table) | n/a (under a mutex) | 3 proofs, symbolic caller ids |
| `kernel/src/scheduler_queue.rs` | `model_scheduler_queue.rs` (vs bounded FIFO, N=2/4/64, threads) | 3 models | FIFO proof, N=2 |
| `kernel/src/ipc_ring.rs` | `model_ipc_ring.rs` (vs bounded FIFO, N=1/2/3/256, MPSC threads) | 3 models | FIFO proof, N=2 |
| `kernel/src/capability_table.rs` (authority v2) | `model_capability_table.rs` (3 domains: create, derive, install, revoke slot/object, destroy, reply caps, incarnation end vs a reference) | 3 models (seqlock slot, racing installs, racing create/revoke), 4 slots | `derive_only_narrows`, `next_generation_moves_and_skips_zero` |
| `kernel/src/ipc_v2/{endpoint,notification,reply}.rs` | `model_ipc_v2.rs` (endpoint, notification, IRQ routes, reply slot vs references) | 5 models (lost wakeup, cancel race, close, park/signal, two signals) | n/a |
| `crates/sex-pdx/src/program_code.rs` (WRPKRU/XRSTOR/XRSTORS scan) | `model_program_code.rs` (vs a byte-by-byte search, random pieces, priming, every offset) | n/a | `key_scan_split_matches_bytewise` (all 10-byte codes, every split; ~11 min, nightly only) |

```sh
cd fuzz/models
cargo test --release                                   # proptest (PROPTEST_CASES=20000 for deep)
RUSTFLAGS="--cfg loom" cargo test --release --test model_loom_queues --test model_loom_v2
cargo kani                                             # all proofs (~20 min); --harness NAME for one
```

Loom needs loom's atomics, so `build.rs` writes copies of the kernel files that swap
`core::sync::atomic` for `crate::shim::atomic`, drop `const` from `new`, build
`[const { .. }; N]` arrays with `core::array::from_fn`, and give the ring's payload a
loom-tracked cell (the endpoint queue too). Spin loops yield to loom. The capability
table copy shrinks to 4 slots and 4 objects and gets a field-by-field constructor
(loom atomics cannot be zeroed heap bytes). Each rewrite must apply or the build fails, so a changed kernel
source is never silently checked as something else. The loom models were
mutation-checked: publishing before writing, a Relaxed sequence load in `steal`, a
Relaxed ring publish and a Relaxed `read_idx` release each fail a model. The new
proptest models were mutation-checked too: dropping the badge check, the stale check
or one owner's destroy in the capability table; an off-by-one queue limit, a broken
cancel shift, a lost re-park in the notification and an unchecked reply generation;
and a wrong offset after the scanner's word skip. Each one fails.

CI: `.github/workflows/assurance.yml` runs small on PRs touching these files and deep
nightly (03:15 UTC), plus a seeded 1M-call syscall and protocol fuzz of the booted
image (`scripts/syscall_fuzz_gate.py`). The seed is the run number; replay a failing
one with workflow_dispatch.

## Isolation matrix (booted image)

`scripts/isolation_matrix_gate.py` (CI job `isolation` in assurance.yml) boots once per
backend: KVM with host PKU (`-cpu host`, skipped when the runner lacks /dev/kvm or
PKU), TCG with PKU, TCG without PKU. `run fuzz iso hold` (program B) names its code,
data and stack addresses; `run fuzz iso read|write|exec ADDR` (program A) touches one
and must be ended by the kernel, B untouched. Also: B's pages after B has ended, and a
copy of the fuzz program with WRPKRU patched over its `KEYH` marker NOP must be
refused at load. Known gaps are written down as `gap` cells, not hidden: no-PKU has no
data isolation until protection views (stream 6), and PKU does not stop instruction
fetch, so calling B's code is a gap everywhere until views.

Capability rows (syscall 105, every backend): B also makes an endpoint and a badged
copy (`[fuzz.iso.cap.hold]`). A deriving from those slot numbers while B runs
(`cross_cap`) and after B ends (`dead_badge`, noting whether A ran in B's row) must be
refused; A making an endpoint, copying it and destroying the root must find the copy
dead for derive and send (`revoked_grant`).

## Findings

- **Reply table entries outlive their counts.** A caller keeps its slot in a callee's
  `ReplyCredits` after its count reaches zero, until `clear` (callee restart or
  retirement). So a callee can be owed by at most `REPLY_CALLERS` (128) distinct
  callers per incarnation, ever. That is safe while the boot registry holds fewer
  callers than 128; past that, new callers get `ERR_SERVICE_NOT_READY` until the
  callee restarts. Pinned by `answered_callers_keep_their_entries_until_clear`;
  the IPC v2 reply caps (plan phase 2) replace this table.

- **Generations wrap after 2^32 - 1 bumps.** An object generation (and a domain
  incarnation) is a u32 that skips 0, so a capability revoked 4 billion revocations
  ago would be honoured again. Unreachable in practice (one owner revoking one
  object 4 billion times, or one domain restarting 4 billion times); pinned by
  `generation_wrap_skips_zero_and_is_the_only_revival` so a counter change is seen.

## Next

- Cross-domain grant rows (OP_GRANT into another domain) once programs can hold a
  Domain capability for a peer.
- Kani proofs over the capability table itself (needs a smaller table under cfg(kani)).
