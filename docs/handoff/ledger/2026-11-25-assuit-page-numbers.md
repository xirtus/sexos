# Assuit dynamic page numbers

- **Gap:** Assuit had no native way to place recurring page numbers in a
  document, though WordCraft supports page fields in headers and footers.
- **Change:** Ctrl+Alt+N opens a bounded prompt for top/bottom placement,
  left/center/right alignment and optional Page X of Y format. It runs
  WordCraft's `insert.pageNumber`, keeping the field dynamic in the document
  rather than typing a fixed number on a page. Reapplying to an engine-generated
  page-number paragraph changes its layout without duplicating the field.
- **Verification:** A focused host test rejects invalid input, checks dynamic
  PAGE/NUMPAGES fields and replacement, then saves and reopens DOCX/ODT. The
  Assuit QEMU gate captures the prompt and numbered page, checks the edit
  receipt, and saves the document. Full build and QEMU run on GitHub CI.
- **Limit:** Removal and section-specific numbering controls are still open.
