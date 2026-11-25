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
The origin note itself is created with a no-replace `CREATE` before writing
its path. Rollback tracks the created note's object ID and removes it only if
that same object still owns the name. This closes a race where a note appeared
after the free-name check and could have been truncated or deleted. The QEMU
gate plants a stranded note, then checks that the new trash entry takes a
different name and the old note keeps its contents.

The Files jobs QEMU gate asserts that the note-sync record precedes the
successful trash record and continues to restore and checksum the item.
GitHub CI must run the full build and QEMU gate at this branch's exact head;
the `files-trash.ppm` screenshot is its visual artifact. Directory moves on
SexFS and restore/undo durability remain separate gaps.
