# Date: edit a personal event time

- T in an open agenda accepts a same-day `HH:MM-HH:MM` range or `all day`. Date keeps UID, calendar, reminders, other details, and CalDAV resource identity; linked events are marked for sync. The staged calendar save and Ctrl+Z/Ctrl+Y history cover each change.
- Date rejects backward or midnight-crossing ranges, multi-day events, recurrence, invitations, and retained conflict notes until those have explicit editing semantics. The model test covers time/all-day, identity, sync state, invalid ranges, and the multi-day refusal.
- GitHub CI's calendar guest gate changes a saved event's time, checks undo/redo, switches it to all day, and captures `date-event-time-edited.ppm`.
