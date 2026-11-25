# Loom graphical grid selection

The single grid now hit-tests pointer clicks against visible tile geometry.
Ctrl-click toggles marked tiles in both single and split grids; Ctrl+Space and
Ctrl+arrows offer the same selection from the keyboard. A focus ring remains
visible when focus moves away from selected tiles. Both layouts pass marked
rows into Loom's existing batch trash, copy and move paths and one-step undo.

The focused `loom-core` test covers tile geometry, margins, gaps, footer and
out-of-range rows. The Loom QEMU gate Ctrl-clicks disjoint grid tiles, captures
`loom-grid-selection.ppm`, copies them as a batch and undoes the group. Full
build and QEMU run only in GitHub CI. Group drag remains unsupported.
