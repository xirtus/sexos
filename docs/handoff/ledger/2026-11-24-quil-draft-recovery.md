# Quil named-file draft recovery

Quil snapshots modified named documents up to 4 MiB to its granted Files folder.
Each snapshot carries its exact path and a content hash. The app stages, syncs,
renames, and syncs a draft; on reopen it restores valid unsaved content into a
modified editor tab without overwriting the original. A committed Save removes
the draft. No ambient paths, POSIX process behavior, or kernel changes are used.

The host codec test rejects truncated, corrupted, and wrong-path snapshots. The
editor QEMU gate restarts Quil after an unsaved edit, verifies recovery, and
captures `quil-draft-recovered.ppm`. Full builds and QEMU run on GitHub CI.
