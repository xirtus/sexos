# Spreadeagle: stop validation for paste and fill

- Ctrl+V and Ctrl+D/R now preview the GridCraft edit on a copy of the workbook. Changed destination cells are checked against the original stop-style rules, including live range sources after the preview. If one value fails, the real command is skipped; cells and undo history stay intact.
- A focused host test checks rejected paste and fill, accepted edits, and undo. The Programs QEMU gate rejects an invalid paste and fill, then captures `spreadeagle-validation-bulk.ppm`.
- External clipboard text and other bulk commands still need validation coverage.
