## Seek ahead: bound the missing-byte wait (2026-11-10)

- Ran the previously untested `program_web_seek_gate.py` on QEMU TCG (ARM macOS host)
  using PR #111 head `4c1ef5c0`, run 42812580544's normal tested ISO. Actual ISO/web
  SHA-256 provenance and failing serial trace are in the local
  `/Users/xmacbookm5/Projects/sexos-evidence/web-seek-pr111/` bundle.
- Failure: video opened early at 593,920 / 1,482,547 bytes; the page sought to eight
  seconds but no `[web.media.range]` appeared. The whole file finished first, with
  `moved after 6833`. The unchanged gate timed out waiting for Range, correctly.
- `Growing::ask` treated a missing gap of up to 1 MiB as near. At 100 KiB/s this can
  wait over ten seconds and covers most of this test clip. Reduce `NEAR` to 128 KiB:
  sequential/small audio-video demuxer gaps still wait, larger missing gaps ask for
  a Range using the existing checked 206/content-range path and kept pieces.
- Three host tests exercise the actual source implementation: this missing forward
  seek, cached old/new piece reads, sequential/boundary gaps, non-Range servers and
  exact end-of-stream. They pass in an isolated portable-source harness on ARM; full
  cargo host tests require x86 because existing sex-pdx inline assembly is x86-only.
- A separate parallel CI job builds an FFmpeg-enabled image and runs the unchanged
  streaming and seek gates plus cargo tests, preserving evidence. Guest validation of the fix remains pending
  that exact candidate build; no timing threshold was loosened.
- No kernel, ABI or authority change. The streaming gate supplies interleaved-playback regression evidence on the
  same feature image; its result is pending.
