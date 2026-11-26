# Quil Git HEAD label

- Branch: `codex/quil-git-branch-label`, stacked on the diagnostic picker PR.
- Gap: the tracked-changes panel did not show which branch or detached commit
  the project is on.
- Change: read only `.git/HEAD` through Quil's existing project Files grant,
  validate the branch ref or detached object ID, and display it above changes.
  Malformed and control-bearing input is shown as unavailable.
- Evidence: focused parser tests, plus the GitHub CI QEMU editor gate checks
  the label and captures `quil-git-branch.ppm`.
- Remaining: branch switching, commits, untracked files, and broad unstaging.
