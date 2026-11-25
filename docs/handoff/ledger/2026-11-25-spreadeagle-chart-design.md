# Spreadeagle chart design controls

- Added native doughnut chart choice and chart editing commands for doughnut hole 10–90% and pie/doughnut first-slice angle 0–359°. Validation rejects unsupported chart types and out-of-range input before GridCraft's undoable `chart.set` action.
- The host test creates a real chart, checks hole and angle change rendered pixels, verifies invalid inputs, undo/redo, and XLSX save/reopen. The Programs QEMU gate edits the budget chart into a doughnut, captures `spreadeagle-doughnut-design.ppm`, and compares it with the earlier presentation screenshot.
- Numeric axis bounds and per-series editing remain unavailable in the current GridCraft model/native command surface. Chart style selection is also left open: a focused test found GridCraft renders doughnut styles 1 and 5 with identical pixels despite storing the style value. Full image build and QEMU proof run in GitHub CI.
