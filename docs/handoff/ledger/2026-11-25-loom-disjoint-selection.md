# Loom noncontiguous text-list selection

Ctrl+Space toggles the focused row; Ctrl+Up/Down/Home/End moves focus while
keeping the selection. Plain movement clears it. Selected paths are deduplicated
and sorted in folder order before they reach the existing batch trash, copy and
move operations, preserving their collision handling, rollback and one-step undo.
The thumbnail grid still has single-item selection.

The focused `loom-core` test covers sparse marks, toggling, range conversion and
out-of-range indices. The Loom QEMU gate captures `loom-disjoint-selection.ppm`
and exercises disjoint copy, trash and cut/move with one undo per operation.
Full build and QEMU run only in GitHub CI.
