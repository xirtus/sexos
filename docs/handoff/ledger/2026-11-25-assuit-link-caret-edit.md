# Assuit: edit a link from its caret

- Ctrl+K with a collapsed caret inside an existing single-paragraph hyperlink selects its whole contiguous label and prefills the current address. Ctrl+A clears that prompt. Enter changes the target and optionally the display text in one WordCraft undo step; an ordinary Ctrl+K selection or insertion still works as before.
- A focused host test checks full-label selection, target and label replacement, one-step undo, DOCX/ODT save/reopen, and plain-text refusal. GitHub CI's Assuit guest gate captures `assuit-link-caret-prompt.ppm` and `assuit-link-caret-edited.ppm`.
- A link spanning paragraphs still needs a broader edit control.
