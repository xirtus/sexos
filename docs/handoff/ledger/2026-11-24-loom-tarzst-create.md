# Loom tar.zst creation

- Gap: Loom could open and extract tar.zst, and its archive engine could write it, but the native file manager exposed only zip and tar.gz creation.
- Change: Ctrl+Shift+Y packs a selected file or folder as tar.zst using Loom's staged archive writer. It shares cancellation cleanup, final rename and Ctrl+Z undo with the other formats. The shortcut is documented in Loom's UI and keyboard reference.
- Test: The Files QEMU gate creates `docs.tar.zst`, browses it, captures `created-tarzst.ppm`, extracts `alpha.txt`, and checks its bytes against the host source. The focused `sex-archive` tar.zst round trip test passed. Full builds and QEMU run only on GitHub CI.
