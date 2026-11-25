# Date: Shell-handled Bell receipts

- Branch: `codex/date-presentation-ack`, stacked on `codex/date-bell-ack`.
- Bell sends a full event ID, Date ticket and source row to Shell before the
  notification chunks. Shell uses the kernel-stamped Bell task and sends a
  handled ACK only after accepting the panel line and history. A user-hidden
  notice settles as muted; a panel route failure returns Retry.
- Bell holds one confirmed request until that exact ACK. It checks event ID,
  ticket, sender, live Bell task and Shell's kernel-stamped domain before
  replying to Date. On Bell restart, pending state is lost and Date's timeout
  retains the durable due minute for retry. Other Bell posts remain one-way.
- Protocol tests cover full-ID round trip and stale Bell task/event refusal.
  The Date QEMU gate requires Shell's handled marker before its screenshot.
  Full build, QEMU and screenshot run only in GitHub CI.
- Remaining: panel enqueue does not prove Display drew pixels or the person saw
  the notice. A lost ACK/receipt can replay it; stable semantic IDs and durable
  cross-restart deduplication need separate work.
