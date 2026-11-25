# Assuit note placement and numbering

- Gap: WordCraft stored note placement and numbering, but Assuit offered no
  native control for those common document settings.
- Change: Ctrl+Alt+Shift+F accepts bounded footnote or endnote options for
  first number, restart, format, and placement. It uses WordCraft's undoable
  `references.noteOptions` command and rejects invalid combinations and ODT
  placements that WordCraft cannot preserve on save.
- Proof: focused DOCX/ODT test checks saved and reopened settings. The guest
  gate captures `assuit-note-options-prompt.ppm` and
  `assuit-note-roman-numbering.ppm`.
- Checks: focused host test, Python syntax and diff check locally; full build
  and QEMU gate on GitHub CI.
