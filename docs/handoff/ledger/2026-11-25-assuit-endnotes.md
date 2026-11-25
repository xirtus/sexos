# Assuit: linked editable endnotes

- Ctrl+Alt+E opens a bounded text prompt at a collapsed body caret. WordCraft inserts a linked endnote and moves the caret into its rich text; Esc returns to the saved body position. The footnote shortcut keeps its existing behavior.
- A focused host test checks input rejection, editability, body return, and DOCX/ODT save/reopen. GitHub CI's Assuit guest gate captures `assuit-endnote-prompt.ppm` and `assuit-endnote-edit.ppm` and verifies insert/return receipts.
- Native note placement, numbering and conversion controls remain open.
