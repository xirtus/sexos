# Spreadeagle chart type and title editing

- Clicking a chart selects its anchor; Ctrl+Shift+Q opens a native prompt to change type among column/line/bar/pie, set a bounded title, or clear it. GridCraft applies the change as an undoable `chart.set` command to the selected chart ID.
- A focused workbook test checks invalid choices, type/title edits, undo/redo and XLSX save/reopen persistence. The Programs QEMU gate edits the chart title and type and captures `spreadeagle-chart-edited.ppm` and `spreadeagle-chart-type-edited.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
