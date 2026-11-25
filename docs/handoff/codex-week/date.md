# Date daily-driver gaps

Compared with GNOME Calendar and Apple Calendar, Date already has month, week and
day views, recurring events, multiple calendars, invitations, reminders through
Bell, world clocks, alarms, timers, `.ics` import/export and two-way CalDAV.

Remaining gaps:

- Saved event, clock and account files truncate the current copy during a write.
  This change stages and syncs them before replacement.
- Account secrets still live in a local `dav.txt` file. Move them into Multipass
  when its account service is ready.
- Scheduling and invitation replies need full interoperability testing with
  several CalDAV providers and mail integration.
- Offline CalDAV conflicts retain local copies in Date's Conflicts calendar;
  `V` now compares each retained copy with the current server event. Resolution
  still requires editing the calendar event and removing the reviewed note.
- The in-app `?` reference and `docs/DATE_SHORTCUTS.md` now cover Date's
  calendar, clock, invitation, sync and export keys. Wider accessibility
  review, including localized key names, remains.
- Date now draws its native surfaces from SexOS light, dark and high-contrast
  appearance tokens, and refreshes when Settings changes theme or accent.
- Date already routes reminders, alarms and timers to Bell. A visible notice can
  now be snoozed for five minutes with Shift+S; the new timer survives restart.
  Date now keeps a bounded durable last-checked minute. When its window closes,
  a hidden Date run takes over reminders, alarms and timers and posts to Bell
  at the due time. An exclusive Files claim prevents duplicate hidden workers;
  reopening Date leaves a handoff marker until the worker exits. A dead worker
  is not automatically restarted while Date stays closed; the durable cursor
  catches up when Date opens again. Simultaneous visible Date windows now keep
  short Files leases and elect one Bell owner; the owner rereads saved clock,
  events and cursor before delivery. Closing one window leaves the other in
  charge; a hidden worker waits until the final window closes. A power cut after Bell posts but before
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
