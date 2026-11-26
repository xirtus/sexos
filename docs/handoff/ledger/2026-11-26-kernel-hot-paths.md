# Microkernel hot paths and the reply queue (Gemini #699, reviewed subset)

From Gemini's `perf/microkernel-hot-paths` (#699), rebuilt on current master.

- **Reply queue instead of one slot**: each domain's `incoming_replies` holds up to
  `MAX_INCOMING_REPLIES` (16) authenticated replies in FIFO order. A second reply while
  one is unread used to be refused and lost (the x17 Spindle `wifi` list bug). A full
  queue still refuses with `BufferFull` before the reply credit is spent.
  **ABI change: a domain can now hold 16 pending replies, not 1.**
- **CoreLocal self pointer** at `gs:[32]` (the old reserved word): `CoreLocal::get()`
  is one load instead of `RDMSR IA32_GS_BASE`.
- **Context switch** skips the FS base and TSC_AUX `WRMSR` when the value on this core is
  unchanged (cached at `gs:[40]`/`gs:[48]`). The only other FS base writer
  (`program_workers` thread pointer set) updates the cache too. FSGSBASE is not enabled,
  so user code cannot change FS behind it.
- **Per-core IRET snapshot** for the GP fault report (`gs:[64..112]`) replaces seven
  global statics written by every core on every switch.
- **Syscall entry** reads PKRU and zeroes it under one `PKU_ENABLED` check.
- Log budgets, first-task and fix markers test with a plain load before the atomic RMW.
- IPC v2 completion with handoff uses `unpark_for_call`. A task parking or polling for an
  IPC v2 answer takes the lean next tick.
- Dead `kernel/src/ipc/buffer.rs` and an empty `kernel/src/alloc.rs` are removed.

Left out on purpose: the per-core active-view cache in `views::enter`. It compared only
the view index, so a view whose root changed would have been skipped. The field stays
reserved at `gs:[56]` to keep the asserted layout.

Pins: `kernel/src/core_local.rs` (whole), `interrupts.rs` and `program_workers.rs`
(wait fingerprints) are re-pinned. Their wait functions are unchanged.
`tests/reply_credit.rs` checks the bounded push, and `scripts/reply_sender_host_gate.sh`
copies `messages.rs` into its mutant tree.
