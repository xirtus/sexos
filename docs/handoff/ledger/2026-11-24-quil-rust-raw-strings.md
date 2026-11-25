# Quil Rust inline raw strings

- Branch: `codex/quil-rust-raw-strings`, stacked on the multi-cursor PR.
- Gap: Rust raw string delimiters were split into plain text, punctuation,
  and quoted spans; `//` or `/*` inside could be drawn as a comment.
- Change: recognize complete one-line `r` and `br` raw strings with up to 16
  hash delimiters as one string span. Unclosed or multiline forms keep the
  existing lexer behavior until a richer cross-line state is added.
- Evidence: focused token test covers raw and raw byte strings containing
  comment markers; GitHub CI QEMU editor gate opens a fixture and captures
  `quil-rust-raw-string.ppm`.
- Remaining: multiline raw strings, longer delimiter runs, and parser-backed
  syntax for Rust, Python, and JavaScript.
