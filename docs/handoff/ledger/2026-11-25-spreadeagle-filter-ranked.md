# Spreadeagle ranked numeric filters

- Added `top N`, `bottom N`, and percent forms such as `top 50%` to the native AutoFilter condition prompt. Counts are bounded to 1–1,000,000 and percentages to 1–100 before GridCraft's ranked filter command. GridCraft includes ties at the threshold and ignores nonnumeric cells.
- A host workbook test checks top and bottom items, percent ranking, invalid input, nonnumeric rows, ties, and XLSX reopen. The Programs QEMU gate filters budget costs to the top two and bottom two, verifies visible rows, captures `spreadeagle-filter-top-two.ppm` and `spreadeagle-filter-bottom-two.ppm`, then clears the filter.
- Combining a ranked filter with a second criterion in the same column remains open. Full native build and QEMU proof run in GitHub CI.
