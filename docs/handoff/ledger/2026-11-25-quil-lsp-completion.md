# Quil language completion

## Gap and change

Quil could publish bounded diagnostics but had no completion request or choice
UI. The bundled language service now advertises and answers
`textDocument/completion` for an open Rust, Python, or JavaScript snapshot. It
returns at most eight keyword labels for the typed prefix. Ctrl+Shift+Space
opens a native choice overlay, and Enter replaces the prefix as an undoable
edit. The editor checks document path, revision, cursor, and single-cursor
state before insertion. The child receives only a bounded snapshot over its
run-scoped pipe; no Files grant is added. This is deliberately keyword
completion, not semantic analysis.

## Verification

- `cargo test --locked -p quil-core`: 39 passed on the host.
- The bundled service has an initialize/open/completion protocol test,
  including wrong-document rejection. The existing GitHub editor job runs
  `quil-core` and `quil-ls` tests and the full QEMU editor gate.
- The QEMU gate requests `ret`, captures `quil-lsp-completion.ppm`, inserts
  `return`, and checks the saved file hash. Build and QEMU evidence are left
  to GitHub CI.
- Corrected the existing Emacs help spelling for `Esc X`, which had caused
  the host shortcut test to fail.

## Limits

The bundled helper has no project-wide symbol index or semantic type context.
Completion is requested on demand for one cursor in a UTF-8 file up to
256 KiB. Invalid and stale responses are refused.
