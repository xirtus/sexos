# Assuit safe page-number removal

- **Gap:** Assuit could add and reposition dynamic page numbers, but could not
  remove one. WordCraft's remove-header/footer commands clear the whole region.
- **Change:** The Ctrl+Alt+N prompt accepts `remove top` and `remove bottom`.
  Assuit runs the undoable engine removal only for a single-section document
  whose target header/footer has one paragraph containing solely the generated
  PAGE field, or PAGE and NUMPAGES fields with the engine's Page X of Y text.
  It refuses other text, extra paragraphs, and different first/even layouts.
- **Verification:** A focused host test covers DOCX/ODT reopen, removal,
  one-step undo, saved removal, both positions, and refusal of an author's
  footer. The QEMU gate removes a newly inserted number, captures the page,
  checks the command receipt, and saves. Full build and QEMU run on GitHub CI.
- **Limit:** Removal from multi-section documents and page-number fields mixed
  with user-edited header/footer content remain outside this safe slice.
