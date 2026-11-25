# Assuit alternate first and even pages

- **Gap:** Assuit could place dynamic page numbers, but could not give the
  first page or even pages their own header/footer layout.
- **Change:** Ctrl+Alt+N accepts `first on/off` for the section at the caret
  and `even on/off` for the document. These run WordCraft's explicit layout
  commands. Enabling a separate slot leaves the default header/footer intact;
  the person can go to that page and insert its own number there.
- **Verification:** A focused host test checks the first page changes when its
  separate slot is enabled and the second page shows a newly inserted even
  footer, checks one-step undo, then verifies both settings through DOCX/ODT
  save, reopen and reset. The QEMU gate captures
  first- and even-page states with command receipts. Full build and QEMU run
  on GitHub CI.
- **Limit:** Assuit still lacks rich header/footer content editing beyond its
  page-number controls.
