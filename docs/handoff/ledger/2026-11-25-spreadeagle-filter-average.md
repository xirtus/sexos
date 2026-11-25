# Spreadeagle average numeric filters

- Added `above average` and `below average` to the native AutoFilter condition prompt. GridCraft calculates the average over numeric cells, including formula results; values equal to the average and nonnumeric cells are excluded from both choices.
- A host workbook test uses 1, 2, 3, a formula returning 2, and text to verify both directions, invalid commands, and XLSX reopen. The Programs QEMU gate filters budget costs above and below average, verifies visible formula rows, and captures `spreadeagle-filter-above-average.ppm` and `spreadeagle-filter-below-average.ppm` before clearing the filter.
- Combining an average filter with another condition on the same column remains open. Full native build and QEMU proof run in GitHub CI.
