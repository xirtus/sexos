# Assuit section page-number format

- **Gap:** Assuit could place, remove, restart and continue dynamic page
  numbers, but could not choose decimal, Roman or letter labels.
- **Change:** The Ctrl+Alt+N prompt accepts `format decimal`, `format lower
  roman`, `format upper roman`, `format lower letter`, and `format upper letter`.
  The engine changes only the section at the caret and keeps its start value
  and header/footer fields. Unsupported format names are refused before any
  document mutation.
- **Verification:** A focused host test checks invalid values, a visible page
  change, section isolation, and DOCX/ODT save/reopen. The Assuit QEMU gate
  captures a Roman-numbered page, checks the edit receipt, and returns to
  decimal. Full build and QEMU run on GitHub CI.
- **Limit:** Alternate first/even headers and some uncommon numbering formats
  remain outside this control.
