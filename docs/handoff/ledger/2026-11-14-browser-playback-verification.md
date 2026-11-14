# Kaleidoscope playback and JIT verification, 2026-11-14

## Established baseline

PRs #117, #195 and #199 are merged. #199 reports the genuine HTTPS embed
at https://xirtus.github.io/sexos/embed-test/ drawing its controls, exchanging
IFrame API messages and fetching AV1 (fmt 396) and Opus from Googlevideo after Play.
That run had no FFmpeg and showed no decoded picture. Fetching streams is not
visible playback. Andreas has reserved visible-playback verification for the x17.

## Prepare the x17 build

Use one clean checkout and keep its commit, dirty diff (if any), complete build
log, ISO hash and `iso_root/programs/kaleidoscope` hash together. Do not copy an
old browser over a newly sealed image. Do not use the release signing key for
this development test.

1. Build `scripts/build_ffmpeg.sh` with the documented musl headers and tools.
   It builds dav1d for AV1 as well as FFmpeg's Opus decoder. Confirm that
   `target/ffmpeg/lib/libavcodec.a` and `libdav1d.a` exist. Preserve the configure
   log and hashes of all archives. A host FFmpeg installation is insufficient.
2. Build the normal sealed image with `scripts/entrypoint_build.sh` in that same
   checkout. `SEXOS_FFMPEG` can name the built prefix; record its absolute path.
   Reject a build log containing `FFmpeg not built` or `media will be refused`.
   Keep the default JIT-off build for the first playback check.
3. Where a QEMU runner is available, first run `program_web_media_gate.py` and
   `program_web_mse_gate.py` with the new ISO and a disposable copy of the seed
   disk. These are codec/MSE fixture regressions, not YouTube or hardware proof.
4. Boot that exact image on the x17 using the existing development-boot procedure.
   Preserve the previous boot image and user data. Record network/display/audio
   devices and screen resolution, and capture the serial log.

## Actual playback acceptance on x17 (pending)

Open the HTTPS embed above through Kaleidoscope. Use its visible Play control
with a physical pointer; do not inject JavaScript, change its user agent or
invent a Referer. Preserve the URL and video identity, and record:

- The genuine player and its controls before Play.
- Googlevideo AV1/Opus responses and `[web.mse.decoder]` with the actual codecs,
  nonzero video dimensions, and no decoder failure.
- Several seconds of visibly changing decoded video *inside the player*, with
  progressing time, then Pause stopping that progression and Play resuming it.
  Use a screen recording or timestamped photographs; a spinner, moving controls,
  thumbnail, downloaded bytes or decoder-open line cannot satisfy this check.
- Audible output, separately from video acceptance; note any underruns/desync.
- A seek and resume, and closing/reopening the player without a crash.

Then repeat on the genuine `/watch` URL. Report embed and watch independently.
If a failure occurs, retain the raw log and screen evidence and identify the
last successful boundary: HTTPS, script, MSE append, decoder, frame delivery,
Display presentation. Never substitute the local tube fixture as a pass.

## Real-page performance (unmeasured)

Use the same opt-in `baseline-jit` browser binary for both modes, launching with
`--no-jit` and without it. The production default remains off. Keep identical
ISO, binary, page/resource bytes, viewport, compiler, network/cache conditions
and interactions. Alternate order across at least three pairs; report every
sample, not only the fastest. Start a fresh browser instance per cold run.

Use ordinary pages (e.g. the browser site suite's encyclopedia, news, link list
and repository page) plus the genuine embed. Verify the visible task outcome
in both modes. Live pages with changing bytes or failed outcomes are diagnostic
runs and cannot establish a speedup. Retained real-page resources can provide
matched replay measurements but must be labelled replay, not live HTTPS proof.

`[web.loaded]` splits fetch/layout/script ticks. `[web.jit]` reports initial
script execution counters, native-tier status and URL; it excludes subsequent
timer, fetch-callback and playback work. Never call that line a whole-page or
playback benchmark. For an interactive task, time the same external start/end
milestones and retain frame/output evidence. Require actual compiled/executed
counts and per-domain code-seal receipts before attributing a difference to JIT.
Pages executing no JIT code still belong in the results, with zero coverage.

Report cold and interactive timings separately, sample dispersion, fallback
counts, correctness differences and environment (QEMU versus x17). No real-page
speedup has been measured in this work block; no speedup is claimed.
