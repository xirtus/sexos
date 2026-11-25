# Quil daily-driver gaps

Compared with VS Code and Zed, the current SDK Quil already has large files,
tabs, a project tree, project search, multi-cursor editing, undo, Vim/Emacs
keymaps, and Rust/Python/JavaScript syntax colors.

Remaining gaps:

- Ctrl+Shift+J now starts a bundled native LSP helper through a run-scoped
  stdin/stdout pipe and shows returned diagnostics. The first helper checks
  delimiter balance and trailing whitespace for Rust, Python and JavaScript
  files up to 256 KiB. Ctrl+Shift+Space now requests up to eight keyword
  completions for one cursor and inserts the chosen label with undo support;
  a document revision and cursor guard rejects stale choices. This is a
  language-service protocol slice, not semantic completion. Continuous
  diagnostics, identifier and semantic completion, cross-file navigation, rename, and
  full language-server implementations remain gaps.
  The diagnostic request now opens a bounded picker with exact source ranges;
  Enter jumps to the selected issue in the same unchanged snapshot. Continuous
  diagnostics remain open.
  Ctrl+Shift+M now requests bounded document symbols from that same child and
  opens a keyboard picker that jumps to a declaration. The helper recognizes
  basic Rust, Python and JavaScript declaration heads; full syntax and
  reference resolution remain gaps. Ctrl+Shift+G now asks the helper for an
  unambiguous declaration in the same file and jumps to it. Cross-file and
  semantic definition resolution remain gaps. Completion now also offers up to
  eight distinct declaration names from the open Rust, Python, or JavaScript
  snapshot before keywords. Scope, import, member, and cross-file completion
  still need a fuller language service.
  Ctrl+Shift+H now requests a bounded plain-text hover summary for a unique
  declaration in the open file. Type inference and documentation hover remain.
- Ctrl+Shift+V shows tracked modified, deleted and conflicted files from a
  checked Git index in the project's Files grant. Right shows a bounded,
  read-only line diff against an authenticated loose Git blob for UTF-8 files.
  S stages a modified tracked UTF-8 file up to 64 KiB in a v2/v3 index with no
  extensions; U undoes only Quil's last stage if the index is unchanged.
  The panel now displays a checked branch name or detached HEAD from `.git/HEAD`.
  Packed blobs, untracked/ignored files, extended indexes, deleted-file staging,
  broad unstaging, commit and branch switching still need native controls.
- Tree-sitter style structural editing, debugging and extensions.
- A save can truncate the live small file, and the large-file path removes the
  original before replacement. This change fixes that data-loss window on the
  same volume. Cross-volume saves need an explicit safe copy strategy.
- Named files and up to eight untitled tabs, each no larger than 4 MiB, now get
  staged, checksummed recovery drafts every two seconds while edited. Restarting
  Quil restores the untitled tabs; reopening a named file restores its draft.
  Saving or explicitly discarding a tab clears its draft. Larger files still
  need recovery. Quil follows the system's light, dark and high-contrast colors,
  including accent changes while open.
- The keyboard reference is available in Quil with Ctrl+Shift+/, including the
  active keymap and scrolling; the complete user guide is `docs/QUIL_SHORTCUTS.md`.
