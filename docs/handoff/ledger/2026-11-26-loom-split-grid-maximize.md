# Loom gate: maximize before the two-pane grid

The files Loom gate timed out waiting for `[loom.split.grid] active=2 pair=1`.
Loom shows the side-by-side tile grids only when each half is at least 144 px
wide and the window fits a full tile row; on CI the Loom window can be tiled
small next to the other boot windows, so Loom fell back to the text split.
The gate now zooms the Loom window (`program_gate_geometry.maximize`) before
turning the grid on. No app change.
