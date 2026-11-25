# Date: edit a personal event title

- E in an open agenda prompts for a new title. Date keeps the event's UID, time, reminders, calendar, and CalDAV resource identity, marks linked events for sync, and saves through its staged calendar write. The change is one Ctrl+Z/Ctrl+Y step.
- Date refuses empty titles, recurring events, invitations, and retained conflict notes until those need explicit decisions. The model host test covers identity, details, sync state, source immutability, and refusals.
- GitHub CI's calendar guest gate changes a saved event's title, checks undo/redo, and captures `date-event-title-edited.ppm`. Full time and location editing remain open gaps.
