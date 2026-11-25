# Assuit gap list

Baseline: a daily word processor such as LibreOffice Writer or Pages. Checked against `apps/assuit` at `aea4ba33a`.

- **Already present:** DOCX and ODT open/save, page layout, paragraph styles, find/replace, undo/redo, PDF export, and recovery copies. The CI QEMU gate opens, edits, saves, reopens, and captures screenshots.
- **Save safety:** ordinary save, Save As, PDF export, and recovery copies stage complete bytes and replace through Files rename. An interrupted Files rename still needs a power-cut gate.
- **Editing UI:** table insertion (Ctrl+T), table row/column selection, insertion and deletion, merge/split cells, cell shading, DOCX table borders and named table styles (Ctrl+Shift+T), image insertion (Ctrl+Shift+I), selected-picture alt text (Ctrl+Shift+R), proportional picture width (Ctrl+Shift+O), and crop percentages (Ctrl+Shift+C) are exposed. Structural edits, shading, alt text and picture formatting have DOCX/ODT round-trip tests. ODT table borders and named table styles are refused because WordCraft drops them on save. A visual table-style gallery remains open.
- **Writing aids:** Ctrl+Shift+Y selects the next issue and offers corrections. A adds the selected issue to Assuit's app-owned persistent dictionary. Ctrl+Shift+K sets the document default and selected text's en-US or en-GB proofing language, preserved in DOCX. A full dictionary manager, more languages, and continuous underlines still need native controls.
- **Print workflow:** PDF export and a selected-page range are now available.
  Letter, Legal and A4 paper with portrait/landscape orientation are now available.
  Margin presets and custom top/bottom/left/right inches are now available.
  A separate print preview remains; the editor already shows the paginated paper.
- **Polish:** the app now follows Settings' light, dark and high-contrast
  appearance for its page desk, status bar and style picker. A keyboard shortcut
  reference is available in the app and `docs/apps/assuit-shortcuts.md`. Verified
  recovery after an interrupted run is covered by a two-boot QEMU power-cut
  gate for idle unsaved edits. This proves the recovery copy survives an abrupt
  stop after Assuit reports it kept the edit; cutting power during a Files
  rename is outside the gate.

Each remaining feature should have a focused host test and a CI guest gate before it is claimed complete.
