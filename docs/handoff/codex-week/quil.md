# Quil daily-driver gaps

Compared with VS Code and Zed, the current SDK Quil already has large files,
tabs, a project tree, project search, multi-cursor editing, undo, Vim/Emacs
keymaps, and Rust/Python/JavaScript syntax colors.

Remaining gaps:

- An LSP client for diagnostics, completion, navigation and rename.
- A Git panel for status, diff, staging, commit and branches.
- Tree-sitter style structural editing, debugging and extensions.
- A save can truncate the live small file, and the large-file path removes the
  original before replacement. This change fixes that data-loss window on the
  same volume. Cross-volume saves need an explicit safe copy strategy.
- Draft recovery needs a complete app-level audit. Quil now follows the system's
  light, dark and high-contrast colors, including accent changes while open.
