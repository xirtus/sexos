# Spreadeagle gap list

Baseline: a daily spreadsheet such as LibreOffice Calc or Excel. Checked against `apps/spreadeagle` at `d48bcc919`.

- **Already present:** XLSX/ODS/CSV read and write, formulas with recalculation, chart creation and rendering, frozen panes, sort, AutoFilter, undo/redo, PDF export, and unsaved-work recovery. The host tests and CI QEMU gate cover these paths.
- **Formula breadth:** a catalog test now requires 200 distinct functions and checks major families. Ctrl+Shift+I searches the engine's live catalog by name, category and description, shows signatures, and inserts a function ready for arguments. A compact ODS/XLSX/CSV import fixture now checks IF, IFERROR, ROUND, AVERAGE, COUNTIF, nested and quoted arguments, and recalculation after source edits. ODS formulas referencing a sheet whose name is normalized on import now follow the imported sheet. A larger Excel/Calc compatibility corpus remains open, especially array and locale-sensitive formulas.
- **Save safety:** normal save, Save As, PDF export, and recovery copies stage and sync complete bytes before Files replaces the destination name. The replacement host test and QEMU power-cut recovery gate cover this path.
- **Workbook UX:** Ctrl+Shift+M offers column, line, bar and pie charts for the selection. Clicking a chart or going to its anchor then Ctrl+Shift+Q changes its type, title, legend placement, data table, axis titles, data labels and gridlines. Ctrl+Shift+J sorts an explicitly selected table by 2–8 columns in priority order. AutoFilter headers offer a native value menu; Space marks several values before Enter applies them, C opens a text/numeric condition, and All rows clears the choice. Ctrl+Shift+K opens a condition directly for the current column, now with two comparisons joined by AND or OR. Warnings returned by workbook import now open in a native review panel and remain available with Ctrl+Shift+W. The native grid still needs deeper chart series, visual styles and numeric axis editing.
- **Grid sizing:** columns and rows can be resized by dragging their header edges. Ctrl+Shift+U now toggles per-cell wrap, the native grid draws multiple lines within the cell, and automatic row heights grow through GridCraft. XLSX and ODS keep the wrap style. General AutoFit for manually sized rows remains open.
- **Polish:** the grid now follows Settings' light, dark and high-contrast colors
  and accent changes. A shortcut reference is in the app and
  `docs/apps/spreadeagle-shortcuts.md`. The Programs QEMU gate now cuts power
  after idle recovery, reboots the same disk, verifies the unsaved cell and
  recalculated totals, saves, and verifies an ordinary reopen.

Claim each remaining feature after a focused host test and CI guest evidence.
