# Spreadeagle compound AutoFilter conditions

- Ctrl+Shift+K now accepts two text or numeric comparisons joined by `and` or `or` for one column, such as `>= 40 and <= 200` or `starts F or contains oo`. It uses GridCraft's native two-condition AutoFilter, so formulas, saved workbooks, and the existing filter menu share the result.
- A focused host test covers invalid input, AND/OR row visibility, numeric ranges, and XLSX reopen. The Programs QEMU gate filters the live workbook and captures `spreadeagle-filter-compound.ppm`.
- Full image build and QEMU evidence run on GitHub CI; focused host, parser, gate syntax and diff checks run locally.
