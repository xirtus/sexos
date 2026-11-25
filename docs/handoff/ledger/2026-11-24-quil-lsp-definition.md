# Quil in-file LSP definition

- Ctrl+Shift+G sends a bounded `textDocument/definition` request over Quil's existing child pipe. The bundled helper returns only one unique declaration in the open Rust, Python or JavaScript snapshot; the client refuses other-file locations and checks the path, revision, cursor and UTF-16 range before jumping.
- Focused core and helper tests cover protocol response, URI scoping, ambiguity and malformed positions. The Quil QEMU gate opens `reference.rs`, follows a use to its declaration and captures `quil-lsp-definition.ppm`.
- This is lexical in-file navigation. Imports, scopes and cross-file references still require a semantic language service. Full image build and QEMU run in GitHub CI.
