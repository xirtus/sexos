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
- Offline edits need a clearer conflict review interface.
- The in-app `?` reference and `docs/DATE_SHORTCUTS.md` now cover Date's
  calendar, clock, invitation, sync and export keys. Wider accessibility
  review, including localized key names, remains.
