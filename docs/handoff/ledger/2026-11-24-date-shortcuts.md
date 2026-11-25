# Date in-app shortcut reference

- Branch: `codex/date-shortcuts` (stacked on `codex/date-crash-safe-save`).
- Gap: Date's keys were only described in source and brief status lines;
  discovering calendar, invitation and clock actions required guesswork.
- Change: `?` opens a scrollable in-app shortcut reference without changing
  the selected view or event. The same commands are documented in
  `docs/DATE_SHORTCUTS.md`. Escape or `?` closes it.
- Test: the calendar QEMU gate opens help, captures `date-shortcuts.ppm`,
  scrolls one row and closes help, checking serial records. GitHub CI runs
  the full build, host calendar tests and QEMU gate.
- Scope: Date view and key handling, gate, shortcut document, gap list and
  this ledger. No kernel, Wi-Fi driver or tether changes.
