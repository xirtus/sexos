# Spreadeagle chart series direction

- Added `switch` to the native chart edit prompt. It invokes GridCraft's undoable row/column series command and reports an error when the chart has no usable source range.
- A host test changes a line chart from one series to three, checks rendered pixels, undo/redo, and XLSX save/reopen. The Programs QEMU gate switches the budget chart, captures `spreadeagle-chart-series-switched.ppm`, compares it with the prior chart screenshot, then switches back before the remaining chart checks.
- Fine-grained series selection and numeric axis bounds remain open. Full image build and QEMU proof run in GitHub CI.
