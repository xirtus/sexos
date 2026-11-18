# IPC fast path and deadline class (2026-11-17, #308)

- **Answers in registers:** syscall 102 `PDX_CALL_RECV` (call, wait, and return the
  answer in registers) and 103 `PDX_REPLY_RECV` (reply, wait, and return the next
  request). `sex_pdx::ipc_recv::{pdx_call_recv, pdx_reply_recv}`. A sleeping caller
  gets its event written into its saved frame when it next runs, so a round trip
  needs no listen after the wake. sex-app's Files requests and Files' own server
  loop use them.
- **Same-core handoff:** the partner of a call or reply runs next on that core, and
  that switch skips the paging-exit poll and admission retry (they run on the next
  ordinary turn, at most 8 handoffs away). A core whose task waits in 102/103 spins
  briefly for the answer before halting, and a wake to a spinning core sends no IPI.
- **Deadline class:** syscall 104 (`sex_pdx::sched::deadline(period_us)`) puts a
  task in earliest-deadline-first scheduling ahead of interactive and normal work,
  bounded to half of each 10 ms window per core. Display (16.7 ms) and Bell (5 ms)
  are in it by default. Host tests: `scripts/scheduler_deadline_host_gate.sh`.
- **Fewer syscalls per Files request:** Files' restricted-I/O poll no longer reads
  the clock and its own epoch three times per loop when it holds nothing, the
  restricted-helper poll reads the clock only when a job waits, and sex-app maps
  its Files region once per run.
- **Proof:** `program_bench_gate.py` ipc_roundtrip at 1 and 4 cores without PKU and
  1 core with PKU (CI `ipc-bench` steps), latency_gate at 1 and 4 cores.
