# Loom batch cut and move

- A contiguous text-list selection can be cut with Ctrl+X and pasted into another folder on the same disk with Ctrl+V. Names already present get Loom's `(copy)` suffix before the extension. Same-folder and descendant moves are refused.
- All source paths are checked before moving. A failed rename rolls earlier moves back. The successful group is one Ctrl+Z step, with metadata moved back together. Cross-disk batch moves and multi-item copies remain open because the current background copy job handles one source at a time.
- Focused `loom-core` tests cover destination policy and collision candidates. The Loom QEMU gate rates one file, moves two files into a new folder, checks that its metadata follows the move, captures `loom-batch-cut.ppm`, checks metadata on undo, and removes the empty folder. Full build and QEMU execution run only on GitHub CI.
