# Spreadeagle AutoFilter menu state

- The native filter menu now reads the active GridCraft criterion when opened. Selected values are checked, conditions are named, and Enter on the untouched All rows row closes without altering the workbook. Space on All rows explicitly clears the active column filter; selecting a different value with the arrows and Enter still replaces it.
- A host workbook test checks selected values, another column's absence of criteria, save/reopen, condition criteria, and clear. The Programs QEMU gate checks active menu state and untouched Enter, captures `spreadeagle-filter-selected.ppm` with the selected value marked, then clears deliberately.
- Large filter value sets still need search and scrolling beyond the existing bounded menu. Full image build and QEMU evidence run in GitHub CI.
