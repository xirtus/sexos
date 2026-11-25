# Loom redo for batch rename and trash

- Added Ctrl+Y for the two file operations Loom can replay from their undo records: batch rename and moving an item to trash. Redo uses the inverse of the recorded rename sequence, including temporary names for cycles; trash redo recreates the origin note before moving the item.
- Redo moves file metadata along with the object. A successful new file operation clears redo, and both undo and redo histories are bounded to 64 steps. On a failed redo, the step remains available to retry.
- The existing Files QEMU gate now undoes, redoes and undoes a trash move and captures `files-trash-redone.ppm`. The Loom QEMU gate does the same for batch rename and captures `rename-redone.ppm`. Both gates continue their file-content assertions afterward.
- Validation: Python syntax, `git diff --check`, focused `loom-core` host test, and full GitHub CI build/QEMU gate. The full build and QEMU run are CI only. Redo for copy, ordinary rename, folder creation and cross-disk move remains open.
