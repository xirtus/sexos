# Streaming fetch bodies and a YouTube-shaped MSE gate

Thread "Play YouTube in Kaleidoscope" (2026-11-19). Continues the YouTube stages
(docs/handoff/YOUTUBE_GAP_REPORT_2026_11_09.md, Y5/Y6/Y8). The cloud can't reach
youtube.com, so this proves on local fixtures the exact shapes the live embed runs saw
(AV1 fMP4 video, itag 39x, plus Opus WebM sound, itag 251, fetched by POST).

## Web platform (crates/sex-js/src/prelude.js)

- WHATWG Streams: `ReadableStream` (default reader, pull/start/cancel, tee, pipeTo,
  pipeThrough, async iteration, `ReadableStream.from`), `WritableStream`,
  `TransformStream`, queuing strategies, `TextDecoderStream` (UTF-8 split across
  chunks) and `TextEncoderStream`. There were none before (`typeof ReadableStream`
  was "undefined"). BYOB readers are refused with a TypeError.
- `Response.body` is a ReadableStream of Uint8Array pieces (16 KiB each); it was null.
  `bodyUsed` follows reads through it. Text from the browser still skips the JS UTF-8
  decoder when nothing touched the stream.
- `new Response(body, init)` works for pages (string, bytes, Blob, URLSearchParams,
  ReadableStream); `Response.error/redirect/json`; `clone()` really copies (it
  returned the same object). `Request` gets a body mixin, signal, clone and the usual
  fields; GET/HEAD with a body throws.
- `fetch` honours `signal`: an abort rejects with AbortError, and the browser's body
  for an aborted load is dropped when it arrives. Blob and stream request bodies.
- Test: `streams_and_fetch_bodies` (crates/sex-js/tests/dom.rs).

## MSE proof

- Fixtures from scripts/web_site/tube/yt/make.sh: AV1 fMP4 with a global sidx, VP9 WebM,
  Opus WebM with Cues first (synthetic pictures and tones).
- Host test `youtube_shaped_streams_and_codec_switch` (crates/sex-web/src/mse.rs).
- QEMU gate scripts/program_web_youtube_gate.py (page tube/yt.html): POST range pieces
  read through `body.getReader()`, chunk-by-chunk appends, `changeType` AV1 MP4 to VP9
  WebM at 4 s with timestampOffset; checks 200 pictures and 4 s each of 600/750 Hz.
- CI media job now runs program_web_mse_gate.py and this gate (timeout 75 min).
