# Loom trash recovery, 2026-11-25

Loom's single-item Delete ignored errors creating trash folders, writing the
restore-path note, and syncing storage. Batch Delete checked note writes but
also ignored final sync. A moved item could therefore be reported as
recoverable without a durable origin note.

Both paths now use one transaction. Loom reserves a name in both the trash and
its note folder, writes and syncs each origin note before moving that item,
then syncs all moved entries before reporting success or offering undo. On an
error it attempts reverse moves, removes notes only for restored items, and
syncs the rollback. An unverified rollback gets an explicit status. File
moves on SexFS use its checked move so a concurrent name claim is not replaced.

The Files jobs QEMU gate asserts that the note-sync record precedes the
successful trash record and continues to restore and checksum the item.
GitHub CI must run the full build and QEMU gate at this branch's exact head;
the `files-trash.ppm` screenshot is its visual artifact. Directory moves on
SexFS and restore/undo durability remain separate gaps.
