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
  System-wide delivery after Date itself closes still needs a userspace service.
- Ctrl+Z/Ctrl+Y now undo and redo local event additions and removals, with each
  result saved immediately. A CalDAV sync clears the history so undo cannot roll
  back server changes. Clock edits and invitation replies still need undo.
- The calendar QEMU gate now kills the guest after a completed staged event save,
  reboots the same disk, and verifies the event in Date. This covers recovery
  after the save receipt; a write interrupted before that receipt remains outside
  the proof.
