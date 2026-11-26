# Quil native LSP diagnostics

Quil can start a hidden `quil-ls` run using Files' existing pipe relay. The
helper gets document text on stdin and returns standard LSP 3.17 framed
JSON-RPC on stdout; it holds no project Files grant or network. Ctrl+Shift+J
requests diagnostics for a Rust, Python or JavaScript buffer and shows the
count and first issue. A 256 KiB bound and timeout keep a slow helper from
holding the editor indefinitely.

The bundled helper checks delimiter balance and trailing whitespace. It is a
first real LSP round trip, not full semantic analysis. Completion, navigation,
rename and continuous updates remain to implement.

Host tests cover fragmented framing and diagnostics. GitHub CI builds the
helper and runs the Quil QEMU gate, which opens an unclosed Rust block, checks
the returned diagnostic and captures `quil-lsp-diagnostics.ppm`.

The sealed build spec explicitly allows `apps/quil-ls/Cargo.toml` alongside
the helper's build stage. The first full CI run caught the omitted allowlist
entry before any QEMU gate ran; the focused build-spec check now passes.
