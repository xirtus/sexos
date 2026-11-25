# Assuit prints selected pages to PDF

- Branch: `codex/assuit-print-range` (stacked on `codex/assuit-shortcuts`).
- Gap: Ctrl+P printed the whole document, with no native way to choose pages.
- Change: Ctrl+Shift+P accepts a 1-based page range such as `1-3,5` and writes
  `name.pages.pdf` beside the source. WordCraft validates the range and retains
  links; staged replacement keeps an existing output safe on write failure.
- Tests: a host test exports page 1 and rejects page 2 of a one-page document.
  The Assuit QEMU gate captures the range prompt, enters page 1, and requires a
  successful selected-page PDF record. Full build and QEMU run on GitHub CI.
- Scope: Assuit book/UI, shortcut reference, QEMU gate, gap list and ledger.
