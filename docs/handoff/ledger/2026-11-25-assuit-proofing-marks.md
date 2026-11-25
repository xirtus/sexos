# Assuit continuous spelling marks

- Ctrl+Shift+U shows or hides WordCraft's real spelling marks on Assuit's editable page. The page cache is invalidated immediately; document bytes and undo history do not change.
- A focused host test draws a misspelled page with marks, without marks, and restored marks, checking the pixels and unchanged text/edit count.
- The Programs QEMU gate toggles the marks around a misspelling and captures `assuit-proofing-marks.ppm` and the hidden-state screenshot. Full build and QEMU execution run on GitHub CI.
- The view choice is session-scoped. Persistent preference and additional proofing languages remain open.
