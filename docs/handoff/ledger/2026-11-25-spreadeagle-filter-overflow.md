# Spreadeagle filter overflow lookup

- The AutoFilter preview stops at 1,000 distinct values or 10,000 rows. A full-range exact-value lookup now exposes a known later value when a typed query has no preview match; Enter applies it. Search accepts spaces, with Ctrl+Space reserved for marking a preview result.
- A generated XLSX fixture has 1,100 distinct rows. The host test confirms the 1,100th value is absent from the preview, resolves case-insensitively, and filters to its row. A menu test checks the exact result is selectable. The Programs QEMU gate opens the fixture, searches `Item 1100`, captures `spreadeagle-filter-overflow.ppm`, and applies the value.
- Partial-match search still sees only the preview, and the extra exact result cannot be combined with other checked values. Full image build and QEMU verification run in GitHub CI.
