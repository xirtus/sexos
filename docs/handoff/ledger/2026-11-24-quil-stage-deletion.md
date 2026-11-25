# Quil stage a tracked deletion

- Branch: `codex/quil-stage-deletion`, stacked on Python interpolation.
- Gap: the Git panel showed deleted tracked files but refused to stage them.
- Change: S removes one absent, unconflicted path from a checked Git v2/v3
  index through the existing staged replacement. U restores the prior index
  only if no other actor changed it. Files that reappear are rejected.
- Evidence: focused index test checks the resulting index, preserved entries,
  absent paths and extensions. GitHub CI QEMU gate stages `other.rs` after
  deleting it and captures `quil-git-deletion-staged.ppm`.
- Remaining: stage new files, evaluate ignore rules, broad unstage, packed
  objects, extended indexes, commits and branches.
