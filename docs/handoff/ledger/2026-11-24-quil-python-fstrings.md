# Quil Python inline formatted strings

- Branch: `codex/quil-python-fstrings`, stacked on the Rust raw-string PR.
- Gap: the Python lexer colored the quote body of an f-string but left its `f`,
  `fr`, or `rf` prefix as ordinary text.
- Change: recognize complete one-line formatted strings and color their prefix
  with the quoted body. Incomplete and triple-quoted strings keep the existing
  behavior until cross-line lexer state is added.
- Evidence: focused token test covers f/fr/rf prefixes and comment-like text;
  GitHub CI QEMU editor gate opens a Python fixture and captures
  `quil-python-fstring.ppm`.
- Remaining: interpolation expression coloring, triple-quoted and multiline
  strings, parser-backed Python syntax.
