# Kernel models (kernel plan phase 7)

Host-only checks of kernel data structures. The tests live at the repo root as
`tests/model_*.rs` and build the **real** kernel sources through `#[path]`; nothing
here is linked into the kernel and no kernel line changes.

| Structure | proptest model | loom | Kani |
|---|---|---|---|
| `kernel/src/ipc/reply_credit.rs` | `model_reply_credit.rs` (vs a map, incl. full table) | n/a (under a mutex) | 3 proofs, symbolic caller ids |
| `kernel/src/scheduler_queue.rs` | `model_scheduler_queue.rs` (vs bounded FIFO, N=2/4/64, threads) | 3 models | FIFO proof, N=2 |
| `kernel/src/ipc_ring.rs` | `model_ipc_ring.rs` (vs bounded FIFO, N=1/2/3/256, MPSC threads) | 3 models | FIFO proof, N=2 |

```sh
cd fuzz/models
cargo test --release                                   # proptest (PROPTEST_CASES=20000 for deep)
RUSTFLAGS="--cfg loom" cargo test --release --test model_loom_queues
cargo kani                                             # all proofs (~20 min); --harness NAME for one
```

Loom needs loom's atomics, so `build.rs` writes copies of the two queues that swap
`core::sync::atomic` for `crate::shim::atomic`, drop `const` from `new`, build
`[const { .. }; N]` arrays with `core::array::from_fn`, and give the ring's payload a
loom-tracked cell. Each rewrite must apply or the build fails, so a changed kernel
source is never silently checked as something else. The loom models were
mutation-checked: publishing before writing, a Relaxed sequence load in `steal`, a
Relaxed ring publish and a Relaxed `read_idx` release each fail a model.

CI: `.github/workflows/assurance.yml` runs small on PRs touching these files and deep
nightly (03:15 UTC), plus a seeded 1M-call syscall and protocol fuzz of the booted
image (`scripts/syscall_fuzz_gate.py`). The seed is the run number; replay a failing
one with workflow_dispatch.

## Findings

- **Reply table entries outlive their counts.** A caller keeps its slot in a callee's
  `ReplyCredits` after its count reaches zero, until `clear` (callee restart or
  retirement). So a callee can be owed by at most `REPLY_CALLERS` (128) distinct
  callers per incarnation, ever. That is safe while the boot registry holds fewer
  callers than 128; past that, new callers get `ERR_SERVICE_NOT_READY` until the
  callee restarts. Pinned by `answered_callers_keep_their_entries_until_clear`;
  the IPC v2 reply caps (plan phase 2) replace this table.

## Next

- Models of the new capability table (authority v2, stream 3) once it lands.
- Loom models of IPC v2 endpoints/notifications (stream 4) when they exist.
