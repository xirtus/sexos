# Date: Bell acceptance receipts

- Branch: `codex/date-bell-ack`, stacked on `codex/date-bell-retry`.
- Due reminders, alarms and timers use a new confirmed Bell post. Bell replies
  after policy and queue admission. A user mute settles the due instance without
  a banner or backlog; spam rate limits, full queue and lost receipts leave
  Date's durable minute pending for retry. Existing one-way posts are unchanged.
- The receipt carries a per-run ticket and is accepted only from Bell's
  kernel-stamped domain. Waiting preserves unrelated window and worker
  messages in the SexOS mailbox. The guest gate now requires Bell's accepted
  receipt before its Date notice screenshot.
- Focused host protocol tests: 2/2 passed (ticket, accepted/muted/retry cursor
  policy, changed chunk).
  Full build and QEMU gate run only in GitHub CI; screenshot pending.
- Remaining: lost receipts can cause replay, and Bell queue acceptance does
  not prove the Shell displayed the notice. Live CalDAV account secrets still
  await Multipass; this change does not claim account integration.
