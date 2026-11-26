# Kaleidoscope reader data tables

- Gap: reader view flattened article tables into loose text, losing row and column relationships needed for reports and comparison articles.
- Change: preserve table captions, rows, header/data cells (including empty cells), links, and bounded column spans from the live DOM. Do not carry site styling, event handlers, or hidden rows into the reader page.
- Proof: focused reader regressions check structure, empty-cell column position, link resolution, hidden-row and hidden-descendant exclusion from output and word count, and unsafe attribute removal. CI now explicitly runs `cargo test --locked -p sex-web --test reader`; its prior host commands skipped this integration target. The GitHub QEMU reader gate now serves a table fixture and captures its reader screenshot.
- Limit: nested tables and row spans are not represented. Full build, QEMU run, and screenshot are pending exact-head GitHub CI.
