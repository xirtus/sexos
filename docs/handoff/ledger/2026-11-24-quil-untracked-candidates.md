# Quil files outside the Git index

- Branch: `codex/quil-untracked-candidates`, stacked on the Git HEAD PR.
- Gap: the Git panel omitted new project files because it only compared index
  entries against the worktree.
- Change: compare the checked index with Quil's bounded project walk and list
  up to 64 distinct files absent from the index as `N` candidates. The panel
  states that Git ignore rules are not evaluated, so it does not claim exact
  untracked status. Existing staging still accepts modified tracked text only.
- Evidence: focused index/candidate tests and the GitHub CI QEMU gate fetching
  a new file, checking the candidate receipt, and capturing
  `quil-git-untracked-candidates.ppm`.
- Remaining: Git ignore/excludes evaluation, precise staged versus unstaged
  status against HEAD, and broader Git operations.
