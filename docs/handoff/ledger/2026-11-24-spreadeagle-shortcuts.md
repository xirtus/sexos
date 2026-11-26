# Spreadeagle shortcut reference

- Branch: `codex/spreadeagle-shortcuts` (stacked on `codex/spreadeagle-theme`).
- Gap: commands were squeezed into the status line with no readable reference
  inside the workbook window.
- Change: add a Ctrl+Shift+/ panel and matching
  `docs/apps/spreadeagle-shortcuts.md`. The panel follows the system theme and
  closes with the shortcut or Esc.
- Test: the Spreadeagle QEMU gate opens and closes the panel, checks serial
  records and captures `spreadeagle-shortcuts.ppm`. The full build and guest
  test run on GitHub CI.
- Scope: spreadsheet UI, QEMU gate, documentation, gap list and this ledger.
