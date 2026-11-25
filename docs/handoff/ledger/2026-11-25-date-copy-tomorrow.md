# Date: copy an occurrence to tomorrow

- In an open agenda, D copies the picked occurrence to the following day and opens that day's agenda. It keeps the title, time, duration, location, description, calendar and reminders, but creates an independent UID and removes recurrence, invitation, conflict and server resource identity. A linked calendar marks the copy New for the next CalDAV sync.
- Ctrl+Z removes the copy and Ctrl+Y restores it. Every step stages and syncs Date's calendar file through the existing save path.
- A focused `sex-calendar` host test covers preserved details, stripped identities, linked/local sync state and source immutability. The existing two-boot calendar QEMU gate now copies an event, checks the changed window screenshot `date-event-copied.ppm`, undo/redo, and persistence after a Date restart. Full build and guest execution belong to GitHub CI.
- The action copies one displayed occurrence as a non-repeating event. Date still lacks arbitrary target dates and editing of existing events.
