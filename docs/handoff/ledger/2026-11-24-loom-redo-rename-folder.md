# Loom redo for ordinary rename and empty folders

- Distinguished F2's same-disk rename from general moves in the undo record, allowing Ctrl+Y to replay it with metadata moved and destination-collision checks. Cross-disk moves remain outside redo.
- Ctrl+Y can also recreate an empty folder after Ctrl+Z removed it. If the path has since been taken, Files returns the collision error and keeps the redo step; undo of a nonempty folder still refuses removal.
- Extended the Files QEMU gate to create an empty folder, rename that selected folder, undo/redo the rename and then undo/redo the creation. It still verifies the copied file's contents and captures `files-rename-redone.ppm` and `files-folder-redone.ppm`.
- Validation: Python syntax, `git diff --check`, and a GitHub CI full image build/QEMU run. Full build and QEMU execution stay on GitHub CI. Redo for copy and cross-disk move remains open.
