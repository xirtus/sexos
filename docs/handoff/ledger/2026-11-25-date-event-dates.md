# Date: copy and move personal events by date

- In an open agenda, Shift+D copies the picked occurrence to a chosen YYYY-MM-DD date. The copy has its own UID and no recurrence, invitation, or CalDAV resource identity. The existing D shortcut still copies to tomorrow.
- M moves a personal, nonrecurring event to a chosen date. It keeps the UID, event details, and server resource identity; linked events are marked for CalDAV sync. Date refuses to move a recurring event, invitation, or retained conflict note until the appropriate series, attendee, or conflict decision is made.
- Each action saves through Date's staged, synced calendar write and becomes one Ctrl+Z/Ctrl+Y history step. The focused model test checks identity, sync state, and refusals. GitHub CI's calendar guest gate checks both actions, undo/redo, and captures `date-event-copy-date.ppm` and `date-event-move-date.ppm`.
- Title, time, location, and recurring-instance editing remain open gaps.
