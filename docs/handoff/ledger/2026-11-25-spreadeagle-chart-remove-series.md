# Spreadeagle remove chart series

- Added `remove N` to the native chart editor, with one-based series numbering, bounds checks, and a guard that keeps at least one series. GridCraft's `chart.selectData` records the edit for undo/redo.
- A host test removes the second of two regional sales series, checks the retained series and changed rendered pixels, rejects invalid numbers and removing the last series, undoes/redoes, and reopens the XLSX. The Programs QEMU gate removes series 2 from a switched budget chart and captures `spreadeagle-chart-series-removed.ppm` before restoring the source layout.
- Editing an individual series' data range and numeric axis bounds remain open. Full native build and QEMU proof run in GitHub CI.
