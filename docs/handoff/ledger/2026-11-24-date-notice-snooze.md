# Date notice snooze

Date already sends reminders, alarms and timers to Bell. This change lets the
person press Shift+S while a notice is shown to ring it again in five minutes.
The snooze is a Date timer in its own crash-safe `clock.txt` file, so a Date
restart keeps it. The existing S key still syncs CalDAV when no notice is shown.

The calendar host test checks save/reopen and a single ring. The Date QEMU gate
starts a two-second timer, verifies that Bell accepts its notification, snoozes
the banner, checks the committed clock file and captures
`date-notice-snoozed.ppm` in `sexos-productivity-proof`.
