# Date daily-driver gaps

Compared with GNOME Calendar and Apple Calendar, Date already has month, week and
day views, recurring events, multiple calendars, invitations, reminders through
Bell, world clocks, alarms, timers, `.ics` import/export and two-way CalDAV.

Remaining gaps:

- The open agenda now copies its picked occurrence to tomorrow with D. The
  copy keeps event details and reminders, gets a fresh UID, strips the source's
  recurrence, invitation and CalDAV identity, and participates in Ctrl+Z/Y.
  A date prompt now supports arbitrary personal-event copy/move. It
  keeps copy identity separate, marks a moved CalDAV event for sync, and makes
  each operation one undo step. Moving a recurring occurrence or invitation
  requires explicit series/attendee semantics and remains outside this slice;
  the agenda now changes a personal event title with E as one undo step;
  the agenda now changes or clears a personal event location with L as one undo
  step. T now sets a same-day time range or all-day status for a personal
  event. Overnight and multi-day time editing still need explicit semantics.
- Saved event, clock and account files truncate the current copy during a write.
  This change stages and syncs them before replacement.
- Account secrets still live in a local `dav.txt` file. Move them into Multipass
  when its account service is ready.
- Scheduling and invitation replies need full interoperability testing with
  several CalDAV providers and mail integration.
- Offline CalDAV conflicts retain local copies in Date's Conflicts calendar;
  `V` compares each retained copy with the current server event. In that review,
  `L` chooses the local copy or `S` keeps the server copy; Enter commits a staged,
  synced calendar save, and Esc cancels. A local choice retains the latest server
  resource address and entity tag, is marked Modified, and reaches CalDAV on the
  next sync. Conflict notes retain their source calendar and resource address; a
  local choice requires that exact server resource. Older notes without this
  identity can only be discarded. If final file sync fails after replacement,
  Date reports uncertain durability and asks for a reopen. If no exact server
  event matches, only discarding the local note is offered.
- The in-app `?` reference and `docs/DATE_SHORTCUTS.md` now cover Date's
  calendar, clock, invitation, sync and export keys. Wider accessibility
  review, including localized key names, remains.
- Date now draws its native surfaces from SexOS light, dark and high-contrast
  appearance tokens, and refreshes when Settings changes theme or accent.
- Date already routes reminders, alarms and timers to Bell. A visible notice can
  now be snoozed for five minutes with Shift+S; the new timer survives restart.
  **Timer transport:** visible Date and its hidden worker now retain due timers
  when their Bell PDX post fails, retrying without losing the saved timer. This
  also applies to reminders and alarms: both delivery paths stop at the first
  failed Bell transport, keep that minute pending and retry after five seconds.
  Date requires a durable initial cursor before delivery. If a fresh cursor
  commit fails, it pauses delivery and retries; on reopen with saved event or
  clock state but no valid cursor, it commits a bounded seven-day catch-up
  baseline before posting. This preserves the first failed notice across a
  window-to-worker handoff when the saved notice state is available, with
  possible replay of older notices in that bounded window.
  A one-time alarm is disabled only after a successful post. Earlier notices
  in the failed minute can replay on retry or after a crash; delivery is at
  least once, not exactly once. A host failure-injection test covers false-then-
  true transport results; the QEMU gate checks healthy background Bell delivery
  and captures its screenshot, but does not force a Bell transport failure.
  Date now posts due reminders, alarms and timers with a Bell receipt. Bell
  answers after deciding what to do with the complete text. A user mute settles
  that due instance without a banner or later backlog; spam rate limits and a
  full queue leave the minute pending for retry. An accepted notice advances
  the cursor only after Bell has kept it. Other apps keep Bell's
  one-way notification call. A lost receipt can replay an accepted notice, and
  Bell acceptance does not prove the Shell displayed it; delivery is still at
  least once. The guest gate checks a healthy Bell receipt and screenshot; it
  does not inject a queue-full or muted Bell state. Host receipt tests check
  muted versus transient refusal cursor policy.
  Date now keeps a bounded durable last-checked minute. When its window closes,
  a hidden Date run takes over reminders, alarms and timers and posts to Bell
  at the due time. An exclusive Files claim prevents duplicate hidden workers;
  reopening Date leaves a handoff marker until the worker exits. A dead worker
  is not automatically restarted while Date stays closed; the durable cursor
  catches up when Date opens again. Simultaneous visible Date windows now keep
  short Files leases and elect one Bell owner; the owner rereads saved clock,
  events and cursor before delivery. Closing one window leaves the other in
  charge; a hidden worker keeps a claim while the window is open and resumes
  from saved state ten seconds after a crashed window's lease expires. The
  elected visible owner now checks due notices during continuous input; a
  non-owner cannot post from its key handler. The
  opening handoff marker also expires after ten seconds. A crashed worker can
  be restarted by the next visible Date launch; no separate system supervisor
  restarts it while Date remains closed. A power cut after Bell posts but before
  the cursor save can replay a notice, so exactly-once delivery is not claimed.
- Ctrl+Z/Ctrl+Y undo and redo local event additions and removals, with each
  result saved immediately. A CalDAV sync clears the event history so undo cannot roll
  back server changes. In Clock, those keys now undo and redo world-zone additions,
  removals and home-zone changes, as well as alarm additions, toggles and removals,
  in one order. A fired alarm invalidates stale undo; live timers are never
  restored. In an invitation
  preview, Ctrl+Z cancels and Ctrl+Y restores the local reply choice; Enter queues the
  `.ics` file. A queued or externally sent reply cannot be recalled. Timer edits
  remain outside undo.
- The calendar QEMU gate now kills the guest after a completed staged event save,
  reboots the same disk, and verifies the event in Date. This covers recovery
  after the save receipt; a write interrupted before that receipt remains outside
  the proof.
