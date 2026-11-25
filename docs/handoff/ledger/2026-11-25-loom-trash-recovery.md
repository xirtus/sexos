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
The origin note uses `CREATE_WITH_DATA`: Files creates a new name, writes the
complete path and commits them in one request, returning the SexFS object ID.
Rollback uses `REMOVE_IF_ID` so a replacement note cannot be removed. Mounted
volumes create and sync the note in one request too; because their formats do
not expose stable object IDs, rollback leaves a note in place and reports
unconfirmed state. The QEMU gate plants a stranded note, then checks that the
new trash entry takes a different name and the old note keeps its contents.
A move request that fails after reaching Files is treated as uncertain: its
note remains for recovery, while earlier confirmed moves are rolled back.
The focused SexFS host regression checks complete note bytes after remount,
an existing-name refusal, and refusal to remove a replacement by the old ID.
CI invokes this integration test explicitly because its existing SexFS repair
step selects only library tests.

The Files jobs QEMU gate asserts that the note-sync record precedes the
successful trash record and continues to restore and checksum the item.
GitHub CI must run the full build and QEMU gate at this branch's exact head;
the `files-trash.ppm` screenshot is its visual artifact. Directory moves on
SexFS and restore/undo durability remain separate gaps.
