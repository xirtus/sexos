# Spreadeagle multiple-value filter

- Added Space toggling to the native AutoFilter value menu. Enter applies all marked values; with no marks it retains the prior highlighted-value behavior. All rows clears the column's filter.
- The workbook method sends a bounded list of distinct values to GridCraft's filter command. A host test checks two visible values, a hidden third row, and clearing. The Programs QEMU gate selects Food and Fun together and captures `spreadeagle-filter-multiple.ppm`.
- Validation: focused workbook test and Python syntax check locally; full image build and QEMU gate only on GitHub CI. No target or disk image is kept locally.
