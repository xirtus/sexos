# Assuit shortcut reference

- Branch: `codex/assuit-shortcuts` (stacked on `codex/assuit-theme`).
- Gap: common commands were squeezed into the status line with no complete
  readable reference inside the app.
- Change: add a Ctrl+Shift+/ shortcut panel and a matching documented table in
  `docs/apps/assuit-shortcuts.md`. The panel follows the system theme and closes
  with its shortcut or Esc.
- Test: the Assuit QEMU gate opens and closes the panel, checks the serial
  records and captures `assuit-shortcuts.ppm`. Full image build and guest test
  run on GitHub CI.
- Scope: Assuit UI, its QEMU gate, documentation, gap list and this ledger.
