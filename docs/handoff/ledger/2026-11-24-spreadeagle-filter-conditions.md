# Spreadeagle filter conditions

- Added Ctrl+Shift+K and C in the native AutoFilter value menu to apply a single text or numeric condition to the current column. Accepted text rules are contains, starts, ends, and notcontains; numeric operators are >, >=, <, <= with finite numbers. GridCraft evaluates the condition against actual cell values, including formula results, and can leave zero visible rows.
- A focused host test checks text, numeric and zero-match conditions and refuses invalid inputs before changing the filter. The Programs QEMU gate applies `contains oo`, captures `spreadeagle-filter-condition.ppm`, and clears it.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
