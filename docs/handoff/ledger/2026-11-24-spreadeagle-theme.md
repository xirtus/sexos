# Spreadeagle follows SexOS appearance settings

- Branch: `codex/spreadeagle-theme` (stacked on `codex/spreadeagle-atomic-save`).
- Gap: the native grid used a fixed light palette even when Settings selected
  dark or high contrast.
- Change: derive grid, heading, formula bar, selection, tab, error and chart
  border colors from shared SexOS theme tokens. Poll the settings generation
  while open and redraw on theme or accent changes.
- Test: the Spreadeagle QEMU gate requires the default Light theme serial
  record and verifies white cells in `spreadeagle-opened.ppm`. GitHub CI runs
  the full image build, host engine tests and QEMU gate.
- Scope: the spreadsheet app, lockfile, its QEMU gate, gap list and this ledger.
