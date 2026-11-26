# Spreadeagle blank filter values

- The native value menu already listed `(blank)`, but its apply path passed an empty string as an ordinary value. GridCraft requires a separate `blanks` flag. The app now sends that flag for blank-only and mixed selections, and names the choice `(blank)` in its status and event record.
- A host workbook test covers a truly empty cell, a formula returning empty text, blank-only and blank-plus-Food selections, clearing the filter, and XLSX save/reopen. The Programs QEMU gate clears a budget category, chooses `(blank)` from the menu, verifies the visible row, captures `spreadeagle-filter-blank.ppm`, then restores the category for later checks.
- The menu's 1,000-value preview and full-range partial search remain separate gaps. Full native build and QEMU proof run in GitHub CI.
