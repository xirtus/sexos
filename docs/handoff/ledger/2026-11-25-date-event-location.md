# Date: edit a personal event location

- L in an open agenda prompts for a new location; Enter on a blank prompt clears it. Date keeps the event's UID, time, reminders, calendar, and CalDAV resource identity, marks linked events for sync, and saves through its staged calendar write. Each change is one Ctrl+Z/Ctrl+Y step.
- Date refuses recurring events, invitations, and retained conflict notes until those need explicit decisions. The model host test covers set/clear, identity, details, sync state, source immutability, and refusal.
- GitHub CI's calendar guest gate sets a location, checks undo/redo, clears it, and captures `date-event-location-edited.ppm`. Time editing remains an open gap.
