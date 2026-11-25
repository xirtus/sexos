# Loom tar.gz creation

- Gap: Loom could browse and extract tar.gz but could only create zip. Finder and Dolphin can package a folder in common compressed archive formats.
- Change: Ctrl+Shift+Z packs a selected file or folder as tar.gz through the existing `sex-archive` writer. It uses the staged `.part` file, cancellation cleanup, final rename, and Ctrl+Z undo shared with zip creation. The app shows Loom branding and documents the shortcut.
- Test: The Files QEMU gate creates `docs.tar.gz`, browses it in Loom, captures `created-targz.ppm`, and checks all three extracted payloads with the host tar reader. `sex-archive` host tests cover tar.gz writing. Full builds and QEMU run only on GitHub CI.
