# IPC v2 endpoints and notifications (2026-11-18, kernel plan phase 2)

- **Objects:** `kernel/src/ipc_v2/` holds the endpoint (one-atomic parked receiver for
  the fastpath, 16-deep caller queue), the one-shot reply slot (cancelled when the
  caller's incarnation moves on) and the notification (64-bit OR word, one waiter).
- **Syscalls:** 105 `SYS_ENDPOINT` (create, send, call, recv, reply, reply_recv, wait,
  grant, derive, destroy) and 106 `SYS_NOTIFY` (create, signal, wait, poll, grant,
  destroy) over authority v2 `Endpoint` / `Notification` capabilities. Every message
  carries the badge minted into the caller's capability. A call that finds its server
  parked wakes it as a same-core handoff (#402). `sex_pdx::endpoint` has the layout,
  constants and wrappers.
- **Registers:** info word + 3 payload words today (the syscall entry saves five
  argument registers); the IPC buffer and the full 8 registers come next.
- **Gates:** `scripts/ipc_v2_host_gate.sh` (state machines, threaded runs, mutants);
  `scripts/ipc_v2_runtime_gate.py` boots and runs `run bench ipc2` on 1 and 4 cores
  (badges, rights, notification bits, destroy, round-trip cycles).
- **Not yet:** IRQ-to-notification binding, Files on endpoints, removing the old
  mailbox path.
