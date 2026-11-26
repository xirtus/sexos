# Quil diagnostic picker

- Branch: `codex/quil-diagnostic-picker`, stacked on the Quil hover PR.
- Gap: the LSP check showed only a count and first message, leaving reported
  source locations inaccessible from the editor.
- Change: Ctrl+Shift+J opens a bounded picker of the helper's diagnostics.
  Up/Down chooses an issue and Enter jumps to its UTF-16 source position.
  The editor rejects a changed document before navigation and refuses malformed
  diagnostic ranges or messages.
- Evidence: the existing helper protocol test publishes a real diagnostic;
  the GitHub CI QEMU editor gate captures the picker and the jump in
  `quil-lsp-diagnostics.ppm` and `quil-lsp-diagnostic-jump.ppm`.
- Remaining: continuous diagnostics and full Rust/Python/JavaScript language
  servers.
