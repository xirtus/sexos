# Assuit section page-number start

- **Gap:** Dynamic page fields could be inserted, but Assuit could not restart
  numbering in a section or continue from the previous one.
- **Change:** Ctrl+Alt+N accepts `start N` for 1–32767 and `continue` for the
  section at the caret. WordCraft's section command updates its page-number
  start without replacing the header/footer field or other section properties.
- **Verification:** A focused host test creates two sections, refuses invalid
  values, confirms only the second section changes, and checks restart and
  continuation through DOCX/ODT save and reopen. The Assuit QEMU gate captures
  the numbered page after `start 5`, verifies the command receipt, then returns
  to continuation. Full build and QEMU run on GitHub CI.
- **Limit:** Number format and alternate first/even header controls remain.
