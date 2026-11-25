# Assuit footnotes

- **Gap:** Assuit had no native way to insert a linked note, though WordCraft could render and serialize footnote stories.
- **Change:** Ctrl+Shift+N opens a bounded footnote prompt at a collapsed body caret. The writer can continue editing the inserted note, and Esc returns to the saved body caret. The note and reference remain owned by WordCraft.
- **Evidence:** A focused host test checks invalid input, the linked reference, editable note text, page rendering and DOCX/ODT round trips. The Assuit QEMU gate captures the prompt and edit mode; full build and guest run happen on GitHub CI.
- **Limit:** Endnotes, note options and navigation among existing notes remain outside this slice.
