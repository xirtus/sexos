# Assuit repeating table headers

- The native Ctrl+Shift+T table prompt accepts `header on` and `header off` for the row at the caret. These run WordCraft's undoable repeat-header command and redraw the editable page.
- A focused host test checks refusal outside a table, on/off behavior, and DOCX/ODT save and reopen of the leading header row.
- The Programs QEMU gate enables the header on an inserted table and captures `assuit-table-repeat-header.ppm`. Full build and QEMU execution run on GitHub CI.
- WordCraft repeats consecutive leading header rows on later pages; marking only a non-leading row does not make it repeat. The existing named DOCX table-style gallery already has preview, selection, save/reopen test, and guest screenshot coverage.
