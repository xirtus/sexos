# Assuit table shading and borders

- Added `shade RRGGBB` for selected cells and `border all`, `outside`, `inside`, or `none` for a DOCX table to Assuit's native Ctrl+Shift+T table prompt. These use WordCraft's undoable table commands. ODT borders are explicitly refused because WordCraft drops them on save.
- A focused host test checks input validation and DOCX/ODT shading round trips, DOCX border persistence, and ODT border refusal. The Programs QEMU gate applies both in DOCX and captures `assuit-table-styled.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
