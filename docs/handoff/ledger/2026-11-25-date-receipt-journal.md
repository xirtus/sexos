# Date: durable reminder receipt journal

- Branch: `codex/date-receipt-journal`, stacked on `codex/date-notice-key-route`.
- Date writes a bounded journal of UID-backed reminder digests only after
  Bell reports Shell handling. Both visible and hidden owners load it on
  takeover, skip a recorded occurrence, and write it before advancing the due
  cursor. The existing staged replacement and sync path protects the old file
  on a failed write; a failed receipt commit retains the due minute for retry.
- Host tests cover failed commit, reopen, duplicate suppression and malformed
  journal input. The Date QEMU gate imports a near-future UID-backed reminder,
  checks Shell key routing and handling, captures its banner, then restarts
  Date and requires a nonempty receipt journal from disk. Full build, QEMU and
  screenshots run in GitHub CI only.
- Shell and Bell have no Files storage authority. The journal closes replay
  after Date's receipt commit, not the Shell-handled-to-Date-commit crash gap.
  It holds at most 256 entries within seven days, and 128-bit digests can
  collide. UID-less events, alarms and timers remain unjournaled. Exactly-once
  delivery is not claimed.
