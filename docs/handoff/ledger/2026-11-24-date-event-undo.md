# Date local event undo and redo

- Added bounded in-session event edit history for additions and removals, including a CalDAV event marked for deletion before sync. Ctrl+Z and Ctrl+Y apply the inverse or redo and save the calendar file.
- Sync and other calendar mutations clear the local history, avoiding rollback of server pulls and invitation replies. The shortcut reference documents the scope.
- Focused model tests exercise addition, deletion, redo, invalidated history, and stale event protection. The Programs QEMU gate adds, undoes, redoes, removes and restores an event, with `date-event-undo.ppm` and `date-event-restored.ppm` screenshots.
- Validation: `cargo test -p sex-calendar --lib --offline` passed (10 tests) with a temporary target removed afterward; Python compile and `git diff --check` passed. Full build and QEMU test run on GitHub CI.
