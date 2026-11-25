# Assuit: explicit cross-paragraph link editing

- Ctrl+K now prefills a link address for an explicit selection across paragraphs when all selected runs carry the same destination. The selection remains unchanged, so changing the address or Ctrl+Alt+K removal uses WordCraft's one-step undoable formatting operation and keeps the words.
- A DOCX/ODT regression selects two linked paragraphs, retargets them, saves and reopens, removes the link, and undoes removal. The test is committed for GitHub CI; it was not run locally.
- Automatic caret expansion stays within a paragraph. WordCraft stores a URL per character run without an identity that could distinguish separate adjacent same-URL links across paragraphs.
- Full build, QEMU screenshot, and exact-head CI result: pending.
