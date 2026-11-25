# Quil keyboard reference

- Added Ctrl+Shift+/ in Quil to open a scrollable shortcut reference for Default,
  Vim and Emacs modes. Rows derive from the keymap tables so they follow bindings.
- Documented the editing shortcuts in `docs/QUIL_SHORTCUTS.md` and updated the gap list.
- Added a keymap host test and QEMU input/screenshot proof in the editor CI job.
- No kernel, driver, or protocol changes. Full build and QEMU verification run on
  GitHub CI.
- The first full Quil host suite found that Emacs Meta keys were displayed in
  lowercase while the reference and its test expect uppercase key labels.
  The reference now displays those keys consistently with other shortcuts.
