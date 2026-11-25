# Spreadeagle: range-backed choice validation

- Ctrl+Alt+L now accepts a current-sheet row or column such as `=D2:D20` as the live source for selected cells. The rule fixes the source coordinates so every destination cell sees the same choices. Blank source cells are skipped, and the native prompt limits the source to 2–1,000 cells.
- A focused host test checks malformed sources, accepted and rejected entries, source edits, and XLSX reopen. The Programs QEMU gate enters a range rule, captures `spreadeagle-validation-range.ppm`, checks an invalid entry, and checks the rule again after reopening the workbook.
- Native dropdown selection and paste/fill validation remain open.
