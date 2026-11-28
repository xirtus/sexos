# 2026-11-28: MediaRecorder and canvas.captureStream in Kaleidoscope (StreamYard stage C3)

Goal (Andreas): Kaleidoscope works with StreamYard. Plan:
docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md. Stacked on C1 (getUserMedia, #863).

## Pages (sex-js)
- `MediaRecorder`: constructor options (mimeType, bitsPerSecond, video/audioBitsPerSecond),
  `start(timeslice)`, `stop`, `pause`, `resume`, `requestData`, `state`, `mimeType`,
  `stream`, bitrates, `MediaRecorder.isTypeSupported`, events start / dataavailable /
  pause / resume / stop / error. `BlobEvent` carries a real `Blob` of the WebM bytes.
  A recorder stops by itself once its stream has no live track.
- `canvas.captureStream(fps)` gives a `CanvasCaptureMediaStreamTrack` (`requestFrame`).
- Tracks carry their source (`camera`, `mic`, `canvas:N`, `audio:N`); only camera and mic
  tracks hold devices.
- A Blob as a video's `src` (object URL) plays as a file (`MEDIA_APPEND ... file`,
  `sex_js::document::MEDIA_FILE`).
- New op `RECORDER = 80` (bytes op; verbs type, start, sources, pause, resume, data,
  stop, take). Dispatches `recorder\tID\t{start|data\tN|error\tNAME|stop}`.
  State: crates/sex-js/src/recorder.rs (at most 16 recorders, 64 MB of pieces waiting).
- Host test: crates/sex-js/tests/recorder.rs.

## Browser (apps/web/src/recorder.rs)
- Pictures from the camera's latest picture or the canvas's own image; sound from the
  microphone (capture.rs `heard_now`), silence when it is quiet or disabled; a disabled
  video track records black.
- At most one picture per recorder per pass of the loop; a pacer (sex-media `Pacer`)
  spaces pictures out when encoding takes more than half the time between them.
- Hook for Web Audio: `Recorders::feed_audio(owner, "audio:N", pcm)` for a
  MediaStreamAudioDestinationNode's track (not called yet; the Web Audio stage does).

## Encoder (crates/sex-media/src/live)
- Streaming WebM, VP8 (libvpx) or VP9 (libvpx-vp9), realtime, plus FFmpeg's own Opus
  encoder at 48 kHz stereo. Clusters of at most a second; `flush` ends one so each
  piece plays after the ones before it. No index, no duration (like other browsers).
- Built only with the export FFmpeg profile (`FFMPEG_RECORDING=2`, which now enables
  `libvpx_vp8` too; scripts/build_libvpx.sh builds VP8). Without it the browser still
  builds and `isTypeSupported` answers false.
- Host test: `SEXOS_MEDIA_HOST_FFMPEG=1 cargo test -p sex-media --test live` builds the
  bridges against the system's FFmpeg (pkg-config) and decodes the recordings back.

## Gates
- Host regressions job: the sex-media live test against system FFmpeg; sex-js tests
  run in the existing sex-js step.
- QEMU: scripts/program_web_recorder_gate.py (page scripts/web_site/recorder.html), in
  the CI `recording` job (the image there has the export profile). With --fake-media it
  records 3 s of camera + mic (VP8) and 3 s of an animated canvas + mic (VP9), checks
  both files with ffprobe and plays each Blob back in a video element.

## Left
- A video element showing a canvas stream directly (`video.srcObject = canvas.captureStream()`)
  stays empty; recording it works.
- mp4/H.264 recording is not offered (isTypeSupported false).
