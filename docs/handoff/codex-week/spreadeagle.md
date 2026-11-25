# Spreadeagle gap list

Baseline: a daily spreadsheet such as LibreOffice Calc or Excel. Checked against `apps/spreadeagle` at `d48bcc919`.

- **Already present:** XLSX/ODS/CSV read and write, formulas with recalculation, chart creation and rendering, frozen panes, sort, AutoFilter, undo/redo, PDF export, and unsaved-work recovery. The host tests and CI QEMU gate cover these paths.
- **Formula breadth:** a catalog test now requires 200 distinct functions and checks major families. Ctrl+Shift+I searches the engine's live catalog by name, category and description, shows signatures, and inserts a function ready for arguments. A larger Excel/Calc compatibility fixture is still needed for argument edge cases and imported formulas.
- **Save safety:** normal save, Save As, PDF export, and recovery copies stage and sync complete bytes before Files replaces the destination name. The replacement host test and QEMU power-cut recovery gate cover this path.
- **Workbook UX:** Ctrl+Shift+M offers column, line, bar and pie charts for the selection. Clicking a chart or going to its anchor then Ctrl+Shift+Q changes its type, title, legend placement, data table, axis titles, data labels and gridlines. Ctrl+Shift+J sorts an explicitly selected table by 2–8 columns in priority order. AutoFilter headers offer a native value menu; Space marks several values before Enter applies them, C opens a text/numeric condition, and All rows clears the choice. Ctrl+Shift+K opens a condition directly for the current column. The native grid still needs deeper chart series, visual styles and numeric axis editing, compound conditions, and clear warnings when import loses unsupported workbook features.
- **Polish:** the grid now follows Settings' light, dark and high-contrast colors
  and accent changes. A shortcut reference is in the app and
  `docs/apps/spreadeagle-shortcuts.md`. The Programs QEMU gate now cuts power
  after idle recovery, reboots the same disk, verifies the unsaved cell and
  recalculated totals, saves, and verifies an ordinary reopen.

Claim each remaining feature after a focused host test and CI guest evidence.
