# Quil same-file identifier completion

- Branch: `codex/quil-identifier-completion`, stacked on the Quil definition PR.
- Gap: the bundled language service offered keywords only, despite already
  recognizing declaration heads for document symbols and same-file definition.
- Change: reuse those bounded declaration names in LSP completion, deduplicate
  them, and keep the editor's existing revision and cursor guard for insertion.
- Evidence: focused Rust, Python, and JavaScript completion tests; the GitHub
  CI QEMU editor gate requests and inserts `helper`, saves the file, and captures
  `quil-lsp-identifier-completion.ppm`.
- Remaining: scope-aware and cross-file completion need a fuller language
  service. This step does not claim semantic resolution.
