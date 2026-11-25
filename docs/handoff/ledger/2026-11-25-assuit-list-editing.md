# Assuit list editing

- Ctrl+Alt+L opens a native list prompt for bullets, numbered formats, list removal, and numbering restart. Each valid choice runs WordCraft's document command and redraws the editable page; the operation participates in undo/redo.
- Focused host tests create a two-item numbered list, save and reopen DOCX and ODT files, and check that both list items remain numbered. They also check restart, removal, and reapplication at the caret, plus the saved numbering definition for every exposed format.
- The Programs QEMU gate opens the prompt, applies numbering, types two items, captures the page in `assuit-numbered-list.ppm`, and saves the DOCX. Full build and QEMU execution run on GitHub CI.
- Nested list level controls remain open.
