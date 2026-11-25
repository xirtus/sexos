# Spreadeagle equality filter conditions

- Added `=` and `<>` comparisons to the native AutoFilter condition prompt, with `!=` as an inequality alias. GridCraft evaluates text, numeric equality, and `*`/`?` wildcards; the parser also accepts these operators as the second comparison in AND/OR rules.
- A host workbook test checks equality, inequality, wildcard matches, an OR join, invalid empty rules, and XLSX save/reopen. The Programs QEMU gate applies `<> Food` and `= F*`, verifies visible rows, and captures `spreadeagle-filter-not-equal.ppm` and `spreadeagle-filter-wildcard.ppm`.
- Literal `*` and `?` escaping remains an open filter input gap. Full native build and QEMU proof run in GitHub CI.
