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
- Five host tests exercise the actual portable production module: this missing forward
  seek, cached old/new piece reads, sequential/boundary gaps, non-Range servers and
  exact end-of-stream. They pass via `scripts/media_source_host_gate.sh` on ARM. The source module is
  re-exported unchanged by sex-media and compiled directly by the host gate, avoiding
  sex-app guest startup/allocator dependencies.
- A separate parallel CI job builds an FFmpeg-enabled image and runs the unchanged
  streaming and seek gates plus the portable source tests, preserving evidence. Local guest qualification and exact-head CI results are
  recorded below; no timing threshold was loosened.
- No kernel, ABI or authority change. The unchanged streaming gate supplies interleaved-playback regression evidence
  on the same feature image.

### Host harness correction

The first new Linux media job exposed a harness mistake before guest testing:
`cargo test -p sex-media` links sex-app's uninitialized guest allocator and aborts on
its first allocation. Move only portable `Source`/`Growing` definitions and their same
three tests to `source.rs`, re-exporting the existing API, and compile that production
module directly in the standalone host gate. All five tests execute; no test or guest
assertion is skipped. The unchanged FFmpeg streaming/seek gates remain required.

### Let a seek discover its byte position

A fresh FFmpeg-enabled sealed build of the smaller-window change (`b053e23a`)
still failed the unchanged guest seek gate: no Range and `moved after 6866`.
The clip is seekable: host ffprobe finds a keyframe at 7.16 s / byte 1,094,999.
FFmpeg can change its sample index during seek while leaving its byte cursor at the
old position until its next read. `Decoder::ready` checked read-ahead at that stale
cursor, preventing the read which would discover the missing target and request Range.

Permit the first decode probe after a successful seek, before that old-cursor preflight.
If it stalls, normal availability/read-ahead checks resume until the requested bytes
arrive; parked Media Source segments retain their existing wait rule. Failed seeks do
not install a pending target. Two additional production-helper tests cover this full
probe/stall/Range/refill sequence and parked-source behavior; all five portable tests
and the Web cross-target check pass. The corrected guest run is recorded below.

### Qualified seek and streaming runs

A sealed FFmpeg/dav1d-enabled image of `55ae0526` passed both unchanged gates on
QEMU TCG, one core, 1280x800 (default wasmi engine at that source revision):

- Seek: opened at 593,920 / 1,482,547 bytes; asked Range from byte 1,094,748;
  advanced to eight seconds after **3,272 ms**, below the unchanged 6,000 ms bound;
  50 remaining frames, no full-file wait, Range refusal, decoder error or guest fault.
- Streaming: opened at 614,400 bytes; 18 pictures shown before download completion;
  all 250 pictures and 480,256 audio frames, with 10.0 seconds of 440 Hz sound.
- Five portable source tests and the Web cross-target check pass. All eleven CI
  checks for `55ae0526`, including the dedicated media job and aggregate build, passed.

Immutable ISO, Web, source hashes, raw serial, WAVs, clips and gate output are in
`/Users/xmacbookm5/Projects/sexos-evidence/pr124-probe-55ae/`. ISO SHA-256:
`a47d5294d1810f3626d5a45c1cd2a95a0835a1be0c75e0bb0a433893f2daea16`;
Web SHA-256 `8b64795b0845d707f132fc5458c99e4343006c7a33163844d26b71dda21a4d13`.
The boot time bound is 900 seconds to allow cold TCG initialization; the actual seek
assertion remains six seconds. This is guest qualification, not hardware playback proof.

The final integration refresh includes the native/JIT engine, Geist and later master
changes. Its CI aggregate now explicitly depends on both `media` and `js-jit`; merely
running the JIT matrix without including it in `build.needs` would leave the required
check able to succeed before JIT acceptance finished. All current-head checks remain
required before merge. New integrated-native evidence will be recorded separately.
