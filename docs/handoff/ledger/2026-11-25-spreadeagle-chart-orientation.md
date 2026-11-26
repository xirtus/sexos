# Spreadeagle chart data orientation

- Branch: `codex/spreadeagle-chart-orientation`, stacked on `codex/spreadeagle-chart-presentation` (#655).
- Gap: a chart's row or column series interpretation could not be changed from the native grid after creation.
- Change: `switch rows/columns` in the selected chart's Ctrl+Shift+Q editor rebuilds the plotted series through GridCraft's undoable chart command.
- Focused host verification: `chart_switch_rows_columns_changes_series_and_round_trips` checks a changed series set and preview pixels, undo/redo, and XLSX save/reopen.
- CI guest evidence: the Programs gate changes a chart's orientation in the actual guest, checks the edit record and captures `spreadeagle-chart-switched.ppm`. Full build and QEMU run on GitHub CI.
- Remaining: series range editing, numeric axis controls, and reliable style preview.
