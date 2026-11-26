# Quil simple Python f-string references

- Branch: `codex/quil-python-interpolation`, stacked on JS template references.
- Gap: the Python lexer colored every f-string expression as literal text.
- Change: color one-line `{identifier}` references separately with punctuation
  braces. Escaped `{{` and complex expressions stay string-colored.
- Evidence: focused lexer test checks plain and keyword references and the
  fallback cases. GitHub CI QEMU editor gate opens a Python fixture and captures
  `quil-python-interpolation.ppm`.
- Remaining: complex expressions, conversions, format specs, nested and
  multiline f-strings, and parser-backed syntax.
