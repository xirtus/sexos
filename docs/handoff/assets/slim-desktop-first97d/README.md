# First native slim desktop evidence

Actual SexOS QEMU captures, not generated concepts. Bundled Geist Sans renders
13px UI titles and the real single-line UTC/date; native body text uses Geist Mono.

- Source: `97d433ce` (`codex/slim-desktop-chrome`), clean tree during fresh build.
- ISO SHA256: `fd75e3415c457806d87e956be04d5e3e4a84b5f3824877ab37ec506ec93f963f`.
- Boot inputs: q35, one TCG core, max/+pku/+rdseed, 512MiB, native VGA, no NIC.
- Fresh Rust target; only Limine bootloader binaries copied. C headers were the
  verified musl headers; Apple Clang21 was selected explicitly. FFmpeg/libc++ were
  absent, so these captures do not qualify media or arbitrary native C behavior.
- PNGs are byte-exact format conversions of QMP screendumps. Each guest was paused
  to pair the pixels with complete serial records and was then stopped/reaped.
- Both 1280×800 and 1024×768: published bar28/title26/inset8/body_top34/target24;
  full3456-pixel clock oracle passed, including anti-aliasing/background erasure,
  Gregorian UTC label and source/admission/completed-publication sequence.
- Separate 2560×1600 scale1→2→1 gate passed matching CPU-publication receipts and
  ink counts2280/18616/2280. Extra 2x capture passed the13824-pixel clock oracle and
  bar56/title52/target48 geometry.

These are first correctness captures. Input/browser regression gates, complete
host qualification, current-head CI and isolated performance qualification remain
required. Whole-panel rounded paint/hit masks are the next separate stage.
Local full evidence: `sexos-evidence/slim-desktop/first97d/`.
