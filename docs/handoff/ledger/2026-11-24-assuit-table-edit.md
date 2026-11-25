# Assuit table editing

- Expose WordCraft's table row and column select, insert and delete commands in Assuit through Ctrl+Shift+T. Commands act on the table cell containing the caret and retain the document engine's undo history.
- Add a DOCX and ODT save/reopen host test for row and column insertion and deletion, including a rejection outside a table.
- Extend the CI QEMU gate with a table edit prompt and edited table screenshots.
- Current limit: cell merge/split, table styling, and image formatting have no Assuit controls. Whole table selection and deletion also remain outside this prompt.
