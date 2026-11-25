# Quil multiline Rust raw strings

- Gap: Quil colored complete one-line Rust raw strings, but a raw string
  crossing lines lost its string color and interpreted comment markers as code.
- Change: carry the raw-string delimiter count in the syntax state between
  lines, including byte raw strings, and resume normal syntax after the
  matching closing delimiter.
- Proof: focused `quil-core` test covers a mismatched delimiter, comment-like
  contents, and ordinary code after closing. The Quil guest gate opens a
  multiline Rust fixture and captures `quil-rust-multiline-raw.ppm`.
- Checks: `cargo test --locked -p quil-core rust_multiline_raw_string`,
  `cargo check --locked -p quil-app --bin quil --target x86_64-unknown-none`,
  Python syntax, and `git diff --check` passed locally. Full build and QEMU
  guest proof are delegated to GitHub CI.
