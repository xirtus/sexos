# Vellum pointer selection of markup

- Branch: `codex/vellum-pointer-annotations`, stacked on signature field placement.
- Gap: existing annotations could only be chosen by stepping through Ctrl+L's list.
- Change: clicking visible markup maps through page zoom, centering and scroll,
  checks its actual highlight quads or annotation rectangle, and opens the
  annotation list with the matching row selected and outlined. Hidden markup
  and PDF widgets or links are excluded; Delete uses the existing undoable path.
- Evidence: focused host test checks separate highlight quads, the gap between
  them and topmost overlap. GitHub CI QEMU edit gate clicks a region highlight,
  captures `pdf-annotation-pointer-selection.ppm`, then deletes and undoes it.
- Remaining: changing annotation colors or appearance, and moving/resizing markup.
