# Assuit linked note navigation

- Gap: linked notes could be edited only immediately after insertion. A saved
  document reopened with no route from its body reference to its note text.
- Change: Ctrl+Alt+Shift+E opens the footnote or endnote under the body caret;
  Esc returns to that body position. Invalid selections and broken references
  are refused. The status line now uses the current note helper.
- Proof: focused DOCX/ODT host test saves, reopens, edits and saves a note, then
  checks both original and new text. The QEMU gate captures
  `assuit-note-reopened.ppm` after editing the referenced note.
- Checks: focused host test, host Assuit check, Python syntax and diff check
  passed locally. The local musl target check could not start because this Mac
  has no `x86_64-linux-musl-gcc`; full build and QEMU proof run on GitHub CI.
