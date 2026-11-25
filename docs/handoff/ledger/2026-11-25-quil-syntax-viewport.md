# Quil syntax state while scrolling

- Gap: multiline Rust raw strings lost their color when the opening delimiter
  scrolled above the viewport. The app reset lexer state at the first shown line.
- Change: each tab retains bounded, versioned syntax checkpoints. It reconstructs
  the first visible line's state, scanning about 64 KiB of ordinary lines per
  draw with a 1 MiB single-line bound. A distant jump shows plain text and "syntax indexing" until exact state
  reaches that line. Editing invalidates the checkpoints. The visible renderer
  also lexes the whole logical line so an opener after drawn column 4096 carries
  to the next line.
- Limit: a line above 1 MiB is shown with "syntax unavailable" instead of
  claiming exact color or allocating its full content for syntax. Full
  parser-backed indexing remains future work.
- Proof: host regressions cover scrolling, distant-jump progress, edit
  invalidation, openers after column 4096, and oversized-line fallback. The
  Quil QEMU gate opens a raw string, jumps to line 90 with its opener offscreen,
  and captures `quil-rust-raw-scrolled.ppm`.
- Full build and QEMU test run only on GitHub CI.
