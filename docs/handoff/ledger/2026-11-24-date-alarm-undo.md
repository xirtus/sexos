# Date alarm undo and redo

- Clock's Ctrl+Z/Ctrl+Y history now orders zone changes with alarm additions, toggles, and removals. Timers remain live, and an alarm that fires invalidates stale history before undo can re-enable it.
- Core tests cover mixed edits, redo, timer survival, and fired-alarm refusal. The calendar QEMU gate adds an alarm, undoes and redoes it, and captures `date-alarm-undone.ppm` and `date-alarm-redone.ppm`.
- Only focused host checks run locally; full image build and QEMU execution run in GitHub CI.
