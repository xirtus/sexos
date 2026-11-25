# Date reversible invitation preview

- A/X in an open agenda now prepares a visible accept/decline reply. Enter queues the local `.ics` file and changes the event; Esc or Ctrl+Z cancels, and Ctrl+Y restores the choice before it is queued. The preview never claims a message was sent.
- Date refuses to overwrite an existing reply file. A queued or sent reply cannot be recalled by local undo; no mail-delivery acknowledgment exists yet. Multi-file atomic commit of the reply and event remains open.
- A focused calendar history test covers choose/cancel/redo/commit. The CalDAV QEMU gate captures `date-invitation-preview.ppm`, cancels and restores the choice, then queues the reply and continues its sync/conflict path.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
