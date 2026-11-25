# Spreadeagle chart layout controls

- Ctrl+Shift+Q at a chart anchor now sets legend placement, horizontal/vertical axis titles, data labels and gridlines through GridCraft's undoable `chart.set` command. The prompt validates choices before changing the workbook.
- A focused host test covers invalid input, undo/redo, and XLSX save/reopen. The Programs QEMU gate logs each native action and captures `spreadeagle-chart-layout-edited.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate in GitHub CI. Local build artifacts are removed after testing.
