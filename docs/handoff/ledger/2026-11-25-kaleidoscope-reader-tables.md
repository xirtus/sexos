# Kaleidoscope reader data tables

- Gap: reader view flattened article tables into loose text, losing row and column relationships needed for reports and comparison articles.
- Change: preserve table captions, rows, header/data cells, links, and bounded column spans from the live DOM. Do not carry site styling, event handlers, or hidden rows into the reader page.
- Proof: a focused reader regression checks structure, link resolution, hidden-row exclusion, and unsafe attribute removal. The GitHub QEMU reader gate now serves a table fixture and captures its reader screenshot.
- Limit: nested tables and row spans are not represented. Full build, QEMU run, and screenshot are pending exact-head GitHub CI.
