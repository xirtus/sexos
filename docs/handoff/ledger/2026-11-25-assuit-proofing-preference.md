# Assuit proofing display preference

- **Gap:** Ctrl+Shift+U could hide continuous spelling marks, but each new
  Assuit run reset to WordCraft's default and showed them again.
- **Change:** Save the view choice in Assuit's own folder with staged, synced
  replacement. Restore it at startup before drawing the document. A failed
  write restores the previous view, and invalid saved data leaves the default.
  The choice does not alter document bytes or the undo history.
- **Verification:** A focused host test covers restart, both choices, invalid
  saved data and failed-write rollback. The existing Assuit QEMU gate now
  toggles marks off, closes and reopens the program, checks its saved-state
  receipt, captures before/after screenshots, then returns to the default for
  the remaining editing checks. Full build and QEMU run on GitHub CI.
- **Limit:** The preference is per Assuit installation, not per document.
