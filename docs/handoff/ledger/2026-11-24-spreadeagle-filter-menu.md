# Spreadeagle AutoFilter value menu

- Added a native value picker to AutoFilter header buttons and Ctrl+Shift+O. The picker lists distinct displayed values from hidden as well as visible rows, supports keyboard and pointer selection, and clears a column with All rows.
- Kept the picker bounded to 10,000 scanned rows and 1,000 distinct values so a very large sheet cannot stall the desktop. Multi-value and condition filters remain open gaps.
- The host filter test checks distinct choices after a row is hidden and switching the selected value. The Programs CI QEMU gate opens the picker, captures `spreadeagle-filter-menu.ppm`, selects Food, clears it, and then continues the existing chart/sort/freeze/reopen path.
- Validation: GitHub CI workflow on this branch (run link in PR). Full image build and QEMU execution are CI only.
