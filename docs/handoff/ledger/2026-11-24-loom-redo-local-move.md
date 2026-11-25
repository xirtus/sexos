# Loom redo for same-volume moves

- Distinguished a synchronous same-volume cut-and-paste rename from a background cross-disk move. Ctrl+Y can replay the former after Ctrl+Z, with occupied-destination checks and metadata following the item.
- The Loom QEMU gate moves `one.txt` from a FAT32 folder into its root, undoes, redoes and undoes the move, captures `same-volume-redone.ppm`, then continues its batch rename and content checks.
- Validation: Python syntax and `git diff --check` locally; full image build and QEMU gate on GitHub CI only. Redo for copies and cross-disk moves remains open because replaying a changed source would give different bytes, and cross-disk moves require a completed background transfer record.
