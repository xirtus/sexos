# Quil declaration hover

- Branch: `codex/quil-lsp-hover`, stacked on same-file completion.
- Gap: Quil could jump to a known declaration, but gave no context at the cursor.
- Change: Ctrl+Shift+H requests `textDocument/hover` from the bundled native
  helper and shows a bounded plain-text declaration kind, name, and line.
  The helper accepts only the open document URI and a unique same-file name;
  the editor rejects a stale document revision or cursor.
- Evidence: helper protocol test and the GitHub CI QEMU editor gate, which
  captures `quil-lsp-hover.ppm` at a real use of `helper`.
- Remaining: full semantic type/documentation hover, scope resolution, imports,
  members, and cross-file symbols.
