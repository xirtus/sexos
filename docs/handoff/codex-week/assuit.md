# Assuit gap list

Baseline: a daily word processor such as LibreOffice Writer or Pages. Checked against `apps/assuit` at `aea4ba33a`.

- **Already present:** DOCX and ODT open/save, page layout, paragraph styles, find/replace, undo/redo, PDF export, and recovery copies. The CI QEMU gate opens, edits, saves, reopens, and captures screenshots.
- **Save safety:** ordinary save, Save As, PDF export, and recovery copy write directly to their destination. An interrupted write can damage the prior copy. First slice: stage a complete file and replace it through the Files rename operation.
- **Editing UI:** table insertion (Ctrl+T), table row/column selection, insertion and deletion, merge/split cells, cell shading and DOCX table borders (Ctrl+Shift+T), and image insertion (Ctrl+Shift+I) are exposed. Structural edits and shading have DOCX/ODT round-trip tests. ODT table borders are refused because WordCraft drops them on save; named table styles, picture sizing/cropping and alt-text editing still need visible controls.
- **Writing aids:** Ctrl+Shift+Y now selects the next issue and offers correction suggestions. Dictionary management, language selection, and continuous underlines still need native controls.
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
