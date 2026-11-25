# Loom plain and gzip tar creation

- Added native Ctrl+Shift+N (plain tar) and Ctrl+Shift+B (tar.gz) actions for the selected file or folder. They reuse Loom's complete staged archive write, cancellation cleanup, undo and archive browsing path.
- `sex-archive` already tests byte-for-byte writer/readback for both output formats. The Files CI QEMU gate now creates and browses each format, captures `inside-tar.ppm` and `inside-tar-gz.ppm`, and checks every member's bytes from the resulting FAT32 stick on the host.
- Copy redo remains a gap: current history retains only the destination path. A safe redo needs content identity checks for the original copy, its undo, and the complete replay so a changed source or edited destination cannot be mistaken for the old copy.
- Validation: focused host archive tests and x86 app check locally. Full image build and QEMU gate only on GitHub CI. No target or disk image is kept locally.
