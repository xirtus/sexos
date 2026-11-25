# Loom plain and gzip tar creation

Ctrl+Shift+F opens a native format picker for ZIP, TAR, TAR.GZ and TAR.ZST.
Plain and gzip tar use `sex-archive`'s existing streaming writer and Loom's
staged `.part` write, completion rename, sync, cancellation cleanup and undo.
The selected item's own Files grant bounds the source and destination; the
folder scan keeps names relative to the selected item's parent.

Focused `sex-archive` round-trip tests verify plain and gzip tar contents.
The Loom QEMU gate creates both formats from a folder, browses each archive,
captures `loom-pack-format.ppm`, and checks every member's bytes with a host
archive reader. Full build and QEMU run only on GitHub CI.
