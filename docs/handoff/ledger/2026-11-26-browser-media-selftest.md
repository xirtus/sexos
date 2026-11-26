# 2026-11-26 Kaleidoscope Browser Media & YouTube Self-Test Suite

## Overview

Added an internal, deterministic Browser Media & YouTube Self-Test suite directly
inside Kaleidoscope (`apps/web`, installed as `/programs/kaleidoscope`) with a
host-testable `#![no_std]` core in `crates/sex-web/src/selftest.rs`.

Zero kernel changes, zero new external crates, and zero large `static` buffers
(all synthetic media streams are generated on the heap only when the self-test runs).

## Invocation on SexOS (x17 hardware or QEMU)

- **Offline / deterministic** (no `+net` required):
  - `run kaleidoscope --selftest` (or `run web --selftest`)
  - Or navigate to `about:selftest` in Kaleidoscope's address bar.
- **Live HTTPS / YouTube probe** (`+net` required):
  - `run +net kaleidoscope --selftest-net` (or `run +net web --selftest-net`)
  - Or navigate to `about:selftest-net` in Kaleidoscope's address bar.
- **Headless / automated exit**:
  - Pass `--selftest-exit` alongside `--selftest` or `--selftest-net` to exit with
    code `0` (all passed) or `1` (any failure) after rendering and emitting the
    serial/tether summary.

## Test Cases

1. **`dom_js_timers`** (`crates/sex-web/src/selftest.rs`):
   - Executes QuickJS-NG (`sex-js`) over a `sex-html` DOM: `document.createElement('video')`,
     `MediaSource.isTypeSupported(...)` across `avc1`, `vp9`, `av01`, `mp4a`, `opus`,
     `setTimeout`, `requestAnimationFrame`, and `onclick` dispatch.
2. **`mse_pipeline` & `mse_sourcebuffer`** (`crates/sex-web/src/selftest.rs`, `apps/web/src/selftest.rs`):
   - Validates the MSE codec support matrix (`avc1`, `vp9`, `vp09`, `av01`, `mp4a`, `opus`,
     plus unsupported negative checks).
   - Feeds heap-synthesized fragmented MP4 (spec-compliant H.264 Baseline `I_PCM` IDR frames)
     and WebM EBML clusters through `sex_web::mse::Stream` and `apps/web/src/mse.rs`
     (`Mse::add`, `append`, `end`, `ready_at`, `first`, `playhead`, `decoders`).
   - Verifies buffered time ranges, gap handling, `timestampOffset`, seek lookup, and live
     FFmpeg frame decode + post-seek timestamp when FFmpeg is linked.
3. **`media_av_sexbell`** (`apps/web/src/selftest.rs`):
   - Synthesizes a short 30-frame H.264 fMP4 video clip and 250 ms 48 kHz stereo `pcm_s16le`
     WAV audio clip on the heap.
   - Exercises `apps/web/src/media.rs` (`Media::open`), `sex_media::Decoder` (decode FPS,
     non-zero FNV-1a frame pixel hash, 500 ms seek accuracy), and `sex_app::sound::Sound`
     (`sexbell` ring write + playback counter).
4. **`yt_net_probe`** (`apps/web/src/selftest.rs`, `--selftest-net` only):
   - Fetches YouTube's lightweight `/oembed` JSON endpoint and watch page over TLS 1.3 via
     `Browser::http`, measures elapsed ms and HTTP status, parses `ytInitialPlayerResponse`
     `streamingData` MIME types, and reports how many discovered video/audio formats are
     accepted by Kaleidoscope's MSE `isTypeSupported`.

## Telemetry & Output

- Emits machine-readable lines to serial/tether log:
  - `[web.selftest] case=<name> status=PASS|FAIL|SKIP ms=<ms> detail=<detail>`
  - `[web.selftest.summary] passed=<N> failed=<M>`
- Renders a structured HTML report table directly inside the Kaleidoscope window.

## Verification

- `python3 scripts/check_architectural_truth.py`
- `cargo test --manifest-path crates/sex-web/Cargo.toml` (host unit tests covering
  DOM/JS/timers/`onclick`, MSE codec matrix, fMP4/WebM pipeline, synthetic WAV/pixel hash,
  YouTube `/oembed` & `ytInitialPlayerResponse` parsers, and summary/HTML formatting)
- `cargo check --manifest-path apps/web/Cargo.toml --target x86_64-sex-app.json -Zbuild-std=core,alloc -Zbuild-std-features=compiler-builtins-mem`
