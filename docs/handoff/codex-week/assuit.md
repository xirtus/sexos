# Assuit gap list

Baseline: a daily word processor such as LibreOffice Writer or Pages. Checked against `apps/assuit` at `aea4ba33a`.

- **Already present:** DOCX and ODT open/save, page layout, paragraph styles, find/replace, undo/redo, PDF export, and recovery copies. The CI QEMU gate opens, edits, saves, reopens, and captures screenshots.
- **Save safety:** ordinary save, Save As, PDF export, and recovery copy write directly to their destination. An interrupted write can damage the prior copy. First slice: stage a complete file and replace it through the Files rename operation.
- **Editing UI:** the engine offers more than the current page view exposes. Table insertion is now available with Ctrl+T and a dimensions prompt; table editing controls and image insertion still need visible controls and round-trip tests for both DOCX and ODT.
- **Writing aids:** no integrated spellcheck or correction UI is visible.
- **Print workflow:** PDF export exists, but page range, paper settings, and print preview are missing.
- **Polish:** the app needs a full shortcut reference, light/dark theme, and verified crash recovery after an interrupted save.

Each remaining feature should have a focused host test and a CI guest gate before it is claimed complete.
