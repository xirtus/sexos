# Quil tracked Git changes

Quil now reads a Git v2/v3 SHA-1 index inside its granted project folder and
compares tracked worktree files with indexed blob IDs. Ctrl+Shift+V shows a
read-only panel for modified, deleted, conflicted and too-large-to-check paths;
Enter opens an existing selection. A damaged index is refused, and the panel
does not claim untracked or ignored status, diffs, staging, commits or branches.

The host test parses a real Git-generated index fixture and checks blob hashes.
The QEMU gate loads that fixture into Quil's Files grant, modifies a tracked
file, checks the panel record and captures `quil-git-status.ppm`. Full builds
and QEMU checks run on GitHub CI.
