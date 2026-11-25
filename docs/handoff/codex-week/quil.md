# Quil daily-driver gaps

Compared with VS Code and Zed, the current SDK Quil already has large files,
tabs, a project tree, project search, multi-cursor editing, undo, Vim/Emacs
keymaps, and Rust/Python/JavaScript syntax colors.

Remaining gaps:

- Ctrl+Shift+J now starts a bundled native LSP helper through a run-scoped
  stdin/stdout pipe and shows returned diagnostics. The first helper checks
  delimiter balance and trailing whitespace for Rust, Python and JavaScript
  files up to 256 KiB. Continuous diagnostics, completion, navigation, rename,
  and full language-server implementations remain gaps.
- Ctrl+Shift+V now shows tracked modified, deleted and conflicted files from a
  checked Git index in the project's Files grant. Untracked/ignored files,
  diffs, staging, commit and branches still need native controls.
- Tree-sitter style structural editing, debugging and extensions.
- A save can truncate the live small file, and the large-file path removes the
  original before replacement. This change fixes that data-loss window on the
  same volume. Cross-volume saves need an explicit safe copy strategy.
- Named files up to 4 MiB now get staged, checksummed recovery drafts every two
  seconds while edited; reopening the file restores its draft. Untitled and larger
  files still need recovery. Quil now follows the system's
  light, dark and high-contrast colors, including accent changes while open.
- The keyboard reference is available in Quil with Ctrl+Shift+/, including the
  active keymap and scrolling; the complete user guide is `docs/QUIL_SHORTCUTS.md`.
