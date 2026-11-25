# Assuit named table style

- Added `style NAME` to Assuit's Ctrl+Shift+T table prompt. WordCraft applies an available named table style at the caret as one undoable command; for example, `style Grid Table 1 Light`. Unknown names or use outside a table return a prompt error.
- A focused host test checks selection and name validation, DOCX save/reopen persistence and explicit ODT refusal because WordCraft drops the style identity. The Programs QEMU gate applies a named style in DOCX and captures `assuit-table-named-style.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only on GitHub CI. No target or disk image is kept locally.
