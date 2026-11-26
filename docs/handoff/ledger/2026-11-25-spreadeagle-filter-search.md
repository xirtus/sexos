# Spreadeagle filter value search

- Added `/` and Ctrl+F search inside the native AutoFilter value menu. Typing narrows its bounded value list case-insensitively; Backspace edits, Esc clears the query, and Enter applies the highlighted match. Checked choices stay tied to original value indices while results change; Space still toggles a result for multi-value filtering.
- A host menu test searches a distant value among 1,000 entries, confirms its original index and that checks survive narrowing and clearing. The Programs QEMU gate searches for `rent`, captures `spreadeagle-filter-search.ppm`, applies Rent, and verifies only its row remains visible before deliberate clear.
- The menu still collects at most 1,000 distinct values and searches only one column at a time. Full image build and QEMU evidence run in GitHub CI.
