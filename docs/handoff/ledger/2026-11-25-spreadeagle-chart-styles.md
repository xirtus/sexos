# Spreadeagle chart styles

- Gap: GridCraft retained chart style numbers in XLSX but its native renderer
  drew a doughnut with styles 1 and 5 identically, and Spreadeagle had no
  style choice.
- Change: Ctrl+Shift+Q accepts `style 1`–`style 16` for a selected chart.
  The GridCraft edit remains one undo step and persists through XLSX; the native
  chart rendering applies distinct palettes while leaving explicit series
  colours alone. These are SexOS native palettes, not exact Excel presets.
- Test: a doughnut's rendered pixels change for style 5, undo and redo restore
  the corresponding pixels, and XLSX reopen keeps both style and appearance.
  The guest gate selects style 5 and captures `spreadeagle-chart-style-5.ppm`.
- Local checks: focused host chart test, Python gate syntax, and diff check.
  Full build and QEMU gate run only in GitHub CI.
