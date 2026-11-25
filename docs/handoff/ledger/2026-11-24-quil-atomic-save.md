# Quil atomic save, 2026-11-24

Quil now streams every edited buffer to a per-task staging file in its own
folder, syncs the complete file, then renames it over the destination and
syncs the rename. It never removes the original before replacement. A failed
write leaves the original intact; a failed rename leaves the staged copy for
recovery. On another volume, Files rejects the rename, and Quil reports the
staged path instead of claiming the save succeeded.

The existing QEMU editor gate tests small and 200 MiB saves and screenshots
the editor. It now checks commit records for both saves. Full build and QEMU
verification run in GitHub CI.
