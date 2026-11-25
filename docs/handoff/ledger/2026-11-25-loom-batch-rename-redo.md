# Loom: redo a batch rename

Base: `codex/loom-trash-recovery` (`5594e2e29417f9db8ab5f0e55bfb3092526f57c5`).

Loom now offers Ctrl+Y after Ctrl+Z takes back a batch rename. Redo replays the
ordered moves, including temporary names for swaps, and restores the same
one-step undo. Any later file action that records undo clears this redo slot;
permanent deletion and emptying trash clear it too. The history stays in the
current Loom run, so it cannot replay stale paths after a restart.

SexFS moves use Files' source-ID and no-replace check. Mounted volumes without
stable IDs first reject an occupied destination, then use Files' rename. That
check is not atomic on those volumes; a concurrent writer remains a gap.
Renames are synced before history advances. A failed move rolls back completed
moves and keeps the current history entry for retry; an uncertain rollback is
reported as a storage error and needs manual folder inspection.

Verification: the `loom-core` host test replays a chain and swap through rename,
undo, and redo. The existing `files_loom_gate.py` QEMU path now exercises
Ctrl+Z, Ctrl+Y, Ctrl+Z on its FAT32 fixture, asserts the redo receipt, and saves
`rename-redone.ppm` for the CI artifact. Full build and guest gate run in GitHub
CI. Other file operations still lack redo.
