# Assuit: remove a selected hyperlink

- Ctrl+Alt+K removes the link from selected text while leaving its words in place. It refuses a collapsed caret or a selection without a link, so an ambiguous multi-word link is not partly changed.
- A focused host test covers refusal, removal, one-step undo, and DOCX/ODT save/reopen. GitHub CI's Assuit guest gate selects the linked word, removes its link, and captures `assuit-hyperlink-removed.ppm` beside the linked-page screenshot.
- Editing an existing link's display text or URL without manually reselecting it remains a gap.
