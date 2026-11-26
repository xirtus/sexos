# Quil document symbols

Quil now asks its bundled language-service child for LSP `textDocument/documentSymbol`
after opening the current UTF-8 Rust, Python or JavaScript snapshot. The native
picker lists at most 128 declarations; Up/Down chooses, Enter jumps, and Esc
closes. The helper receives text through Quil's run-scoped pipe, with no Files
grant. A version and path check rejects an outline after the document changes.

The first helper recognizes declaration heads, not a full syntax tree. It does
not resolve references or rename symbols. Files over 256 KiB are excluded.

Verification: host tests cover symbol extraction, bounds, LSP request/response
and URI isolation. The GitHub QEMU editor gate captures the outline and jump
screenshots. Full image and QEMU verification run on GitHub CI only.
