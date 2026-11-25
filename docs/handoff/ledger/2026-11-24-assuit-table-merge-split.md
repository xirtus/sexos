# Assuit table merge and split

- Added `merge cells` for a selected table range and `split 2` through `split 10` for the current cell to Assuit's native Ctrl+Shift+T table prompt. WordCraft executes both as undoable document commands.
- A host test merges adjacent cells, saves and reopens DOCX and ODT, then splits the cell and checks the saved result. The Programs QEMU gate selects a row, merges it, splits it, and captures `assuit-table-merged.ppm` and `assuit-table-split.ppm`.
- Validation: focused host test and Python syntax check locally. Full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
