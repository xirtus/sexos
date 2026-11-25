# Date: route reminder identity to Shell

- Branch: `codex/date-notice-key-route`, stacked on `codex/date-notice-dedupe`.
- UID-backed reminders send a stable 128-bit digest before their keyed Bell
  chunk stream. Bell binds it to the kernel-stamped sender task and Date ticket;
  a missing or mismatched key returns Retry. Bell forwards it with the full
  event ID, and Shell requires the matching Bell task and event before it can
  acknowledge the completed text. The key is observable in Shell's serial log.
- Host protocol tests cover digest stability, malformed BEGIN sender words,
  stale Bell event/task and mismatched Date task/ticket. The Date QEMU gate
  imports a near-future UID-backed reminder, requires Shell's semantic key
  marker and matching handled event, then captures the visible reminder.
  Full build, QEMU and screenshot run on GitHub CI.
- Shell does not persist the digest or suppress a repeated notice. Bell and
  Shell histories remain volatile; the delivery contract stays at least once.
  Legacy UID-less events, Clock alarms and timers still need source identities.
  A 128-bit digest also has a collision risk, so any durable dedupe store must
  define its collision policy and commit-before-ACK order.
