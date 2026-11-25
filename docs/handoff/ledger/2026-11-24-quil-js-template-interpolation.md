# Quil simple JavaScript template substitutions

- Branch: `codex/quil-js-template-interpolation`, stacked on Python f-strings.
- Gap: the JavaScript lexer colored a complete backtick template as one string,
  hiding even a simple `${name}` reference.
- Change: separate one-line `${identifier}` substitutions into punctuation and
  identifier spans. Escaped `${` and complex expressions stay string-colored;
  no nested or multiline template claim is made.
- Evidence: focused lexer test checks plain/keyword references, escapes, and
  complex fallback; GitHub CI QEMU editor gate opens a JavaScript fixture and
  captures `quil-js-template-interpolation.ppm`.
- Remaining: nested/complex expressions, multiline templates, and parser-backed
  JavaScript syntax.
