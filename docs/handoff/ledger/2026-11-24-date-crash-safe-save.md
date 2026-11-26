# Date crash-safe saves, 2026-11-24

Date now writes events, clock settings and the imported CalDAV account to a
staging file in its own folder, syncs that file, then renames it over the live
file and syncs the rename. A failed write leaves the previous complete file
intact. An interrupted write can leave a staging file, which a later save
replaces. The ordinary calendar QEMU gate creates an event and confirms it
survives closing and reopening Date; it also checks the new commit record.

Full build and QEMU verification run in GitHub CI.
