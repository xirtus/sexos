# Assuit direct list-level keys

- **Gap:** Nested levels were available only through the Ctrl+Alt+L text
  prompt, interrupting keyboard editing of an outline.
- **Change:** Ctrl+Alt+Right moves the list item at the caret one level deeper;
  Ctrl+Alt+Left moves it one level shallower. The gestures refuse plain
  paragraphs, selections and movement past levels 1–9. They use WordCraft's
  undoable paragraph command and its document model.
- **Verification:** A host test checks boundaries, one-step undo, unaffected
  sibling item, and DOCX/ODT persistence. The Assuit QEMU gate exercises both
  keys, checks the edit records, and captures page screenshots. Full build and
  QEMU run on GitHub CI.
- **Limit:** Mixed multi-paragraph selections still use the level prompt;
  custom list definitions remain outside this slice.
