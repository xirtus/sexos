# Loom tar.zst creation

Ctrl+Shift+M packs the selected file or folder as tar.zst beside it. Loom uses
sex-archive's existing TarZst writer and the same hidden part file, rename,
sync, cancellation, and undo path as ZIP creation. No new capability is needed.

The Loom CI guest gate creates and browses an archive, captures
`inside-tar-zst.ppm`, and checks each file on the host after zstd decompression.
Full builds and QEMU testing run only on GitHub CI.
