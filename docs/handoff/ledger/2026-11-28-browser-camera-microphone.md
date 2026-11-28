# 2026-11-28: camera and microphone for web pages (getUserMedia), stage 1 of StreamYard

Goal (Andreas): Kaleidoscope can use the mic and camera, for sites like StreamYard.
Plan for the whole road (this is stage C1): docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md.

## Pages (sex-js)
- `navigator.mediaDevices`: `getUserMedia`, `enumerateDevices`, `getSupportedConstraints`,
  `ondevicechange`; `getDisplayMedia` rejects NotSupportedError for now.
- `MediaStream`, `MediaStreamTrack` (enabled, muted, stop, clone, getSettings,
  getConstraints, getCapabilities, applyConstraints, ended/mute/unmute events),
  `MediaDeviceInfo`, `InputDeviceInfo`, `MediaStreamTrackEvent`, `PermissionStatus`.
- `video.srcObject = stream` shows the camera; `canvas.drawImage(video)` draws any
  video's current picture (files too).
- `navigator.permissions.query({name: 'camera' | 'microphone'})` reads the real choice.
- New text op `CAPTURE = 40` (verbs secure, ask, use, devices, state); dispatches
  `capture`, `capturegone`, `capturemute`, `devicechange`. Shared state
  `sex_js::Capture` (per-site decisions, device list). Only secure pages may ask
  (https, http on localhost); iframes may not (no `allow=` delegation yet).
- Host test: crates/sex-js/tests/capture.rs.

## Browser (apps/web)
- apps/web/src/capture.rs: the browser's own question card under the omnibox
  ("site wants to use your camera and microphone", Allow / Block, Escape = not now),
  drawn by sex-web `Chrome::permission` (`PermissionPrompt`, `permission_rects/hit`).
  Pages cannot draw over it. Answers kept in Files `Kaleidoscope/site-permissions.tsv`
  (not in private windows).
- Webcam: SexUVC's live frame bank (same as Flash), OP_CLIENT_START/STOP so the camera
  (and its LED) runs only while a live track uses it; MJPEG/YUY2/NV12 decoded with
  sex-uvc, fitted to 1280x720. Picture shared as `video:N` for every stream element.
- Microphone: new `sex_app::sound::Listener` (listens on the program's ring beside a
  playing player; re-asks if a player reopens the ring). Bell still asks the person per
  program; until then the track is muted and the card says
  `volume ROW mic on` in Spindle.
- Omnibox shows a red dot with camera / mic icons while the shown page uses them.
- Devices stop when no live track uses them, when the page navigates away, and on exit.
- `--fake-media` (gates): test pattern camera (640x480 YUY2 through the real decoder),
  440 Hz test microphone, plain http pages may ask.
- QEMU gate: scripts/program_web_capture_gate.py (CI, browser daily job).
