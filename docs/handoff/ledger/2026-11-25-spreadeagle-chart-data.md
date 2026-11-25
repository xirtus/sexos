# Spreadeagle chart data range

- Added `data A1:B4` to the native chart edit prompt. GridCraft rebuilds the selected chart's series from the supplied range through its undoable `chart.selectData` command; malformed, empty, or overlarge ranges return an error.
- The host test extends a chart from two to three months, checks the series reference and changed rendered pixels, undoes/redoes, and reopens the XLSX. The Programs QEMU gate extends the budget chart through row 5, captures `spreadeagle-chart-data-expanded.ppm`, compares it with the earlier chart, and restores the original range.
- Fine-grained series editing and numeric axis bounds remain open. Full native build and QEMU proof run in GitHub CI.
