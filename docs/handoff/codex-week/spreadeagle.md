# Spreadeagle gap list

Baseline: a daily spreadsheet such as LibreOffice Calc or Excel. Checked against `apps/spreadeagle` at `d48bcc919`.

- **Already present:** XLSX/ODS/CSV read and write, formulas with recalculation, chart creation and rendering, frozen panes, sort, AutoFilter, undo/redo, PDF export, and unsaved-work recovery. The host tests and CI QEMU gate cover these paths.
- **Formula breadth:** a catalog test now requires 200 distinct functions and checks major families. A larger Excel/Calc compatibility fixture is still needed for argument edge cases and imported formulas.
- **Save safety:** normal save, Save As, PDF export, and recovery copies write directly to the destination. First slice: stage, sync, and replace through Files' rename.
- **Workbook UX:** Ctrl+Shift+M now offers column, line, bar and pie charts for the selection. AutoFilter headers now offer a native single-value menu with All rows. The native grid still needs a full formula picker, chart editing, multi-column sort, multi-value/condition filter choices, and clear warnings when import loses unsupported workbook features.
- **Polish:** the grid now follows Settings' light, dark and high-contrast colors
  and accent changes. A shortcut reference is in the app and
  `docs/apps/spreadeagle-shortcuts.md`. The Programs QEMU gate now cuts power
  after idle recovery, reboots the same disk, verifies the unsaved cell and
  recalculated totals, saves, and verifies an ordinary reopen.

Claim each remaining feature after a focused host test and CI guest evidence.
