# Abacus extras: round 176

Abacus's Weft window now has Scientific, Programmer and Convert mode buttons and
matching menu/palette commands. It depends on the shared Weft/Abacus base in #220;
no toolkit or kernel code changes are needed.

Scientific mode retains trigonometric/log functions, degrees/radians, factorial,
constants and `ans`. It adds hyperbolic functions, `exp`, `exp2`, `log2`, `cbrt`
and `trunc`. Nonfinite results are errors, and expressions/nesting are bounded.

Programmer mode uses an independent exact u64 parser, never f64. Choose unsigned
8/16/32/64-bit words and decimal/hex/binary/octal output in the Words menu or
command palette. Arithmetic wraps at word width; right shifts are logical. Inputs
may use decimal, `0x`, `0b`, `0o`, underscores, `ans`, parentheses, unary `~`/`-`,
`+ - * / %`, `<< >>`, `& ^ |`. Precedence follows those conventional operator
classes; `^` is XOR here and exponentiation only in Scientific mode. Divide/modulo
by zero and shifts at or above word width fail visibly. Extra Weft buttons expose
hex digits and bitwise operators; scientific-only buttons are disabled.

Convert mode accepts `VALUE FROM to TO`, for example `32 F to C` or
`1 MiB to KiB`. Units are case-sensitive and dimension checked:

- Length: m, km, cm, mm, in, ft, mi.
- Mass: kg, g, lb, oz.
- Time: s, ms, min, h, day.
- Temperature: K, C, F, including offsets and absolute-zero validation.
- Storage: B, kB, MB, GB (decimal); KiB, MiB, GiB (binary).

The bounded 500-row history retains each result's mode and programmer word/radix
context. Double-click/Enter on a row, or Recall Last Calculation (`Alt+Up`), restores
that context and places a reusable value in the expression. Conversion recalls the
original conversion expression. Recall replaces existing input. Clear History
clears contexts too. History is session-local; it is not persisted across reboot.

Host validation includes exact integers above f64's precision, wrapping/precedence,
error bounds, affine temperatures, binary/decimal storage, scientific domains, real
Weft mode buttons, history restoration and widget painting. Existing keypad tests
remain intact. The QEMU Weft gate additionally switches modes through the palette,
checks 8-bit overflow, full-width hexadecimal output, temperature/storage conversion,
recall, a dimension error and a scientific function, preserving screenshots.
A guest build does not establish runtime success: that gate must pass in CI.
