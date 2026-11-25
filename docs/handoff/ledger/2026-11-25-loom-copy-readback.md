# Loom staged copy readback

Every background CopyFile now keeps its hidden `.part` file until a size check
and incremental SHA-256 readback match the bytes read from the source. A read
failure, short read or hash mismatch routes through the job's existing cleanup:
the staged file and all earlier destinations in a batch are removed, while
source items remain. Sync happens before readback and promotion. The progress
line says Verifying while a staged file is checked.

Focused `loom-core` tests inject a changed byte, truncated data, an oversized
file and a read failure. The Loom QEMU gate asserts verification logs for both
batch copies, captures `loom-verified-copy.ppm`, checks completed copies and
one-step undo, and the host folder check excludes leftover `.part` files.
Full build and QEMU run only on GitHub CI. This does not complete cross-disk
batch cut: source-trash commit, crash recovery and cross-disk undo remain.
