# Quil wrapped multi-cursor selection

- Branch: `codex/quil-occurrence-wrap`, stacked on the Git candidate PR.
- Gap: Ctrl+D stopped when its wrapped search met an already selected match,
  leaving later unselected occurrences before the starting word.
- Change: advance past selected hits until finding a new occurrence or exhausting
  the one wrapped scan. The existing multi-cursor edit and undo paths remain in
  use.
- Evidence: focused core test starts at the third of four matches, selects all,
  replaces them, and undoes. The GitHub CI QEMU editor gate repeats selection,
  captures `quil-multicursor-wrap.ppm`, and checks the saved bytes.
