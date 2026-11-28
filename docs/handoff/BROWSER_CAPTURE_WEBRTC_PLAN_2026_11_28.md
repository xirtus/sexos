# Camera, microphone and WebRTC in Kaleidoscope (2026-11-28)

Goal (Andreas): the browser uses the mic and camera with sites like StreamYard.
StreamYard (and Meet, Jitsi, Zoom web, Discord) need four things, in this order:
capture (getUserMedia), Web Audio (mic level meters, mixing), recording
(MediaRecorder), and a WebRTC peer connection that sends the camera and mic to the
site's media servers. Stages below; each one lands as its own PR with a QEMU gate.

Design rules kept: all native SexOS (no Linux, no PulseAudio/V4L2 shims). Devices are
owned by their servers (SexUVC, Bell); the browser is a client. Pages only ask; the
browser asks the person in its own card, and Bell still asks once per program.

## C1 Capture (this PR)
getUserMedia, MediaStream/MediaStreamTrack, enumerateDevices, permissions, per-site
card, camera into `<video>` and canvas, Bell mic listener, `--fake-media`.
See docs/handoff/ledger/2026-11-28-browser-camera-microphone.md.

## C2 Web Audio with the microphone
- `AudioContext` / `OfflineAudioContext` (sex-js, graph in the browser, not in JS):
  destination, GainNode, AnalyserNode (StreamYard's level meter), OscillatorNode,
  AudioBufferSourceNode, BiquadFilter, ChannelSplitter/Merger, DynamicsCompressor,
  MediaStreamAudioSourceNode, MediaStreamAudioDestinationNode, MediaElementSource,
  decodeAudioData (sex-media), AudioWorklet later.
- Graph runs in apps/web (fixed point where floats cost), output through the
  program's Sound ring; the microphone comes from capture's Listener frames.
- Gate: analyser sees the fake 440 Hz tone at the right bin.

## C3 MediaRecorder
- `MediaRecorder` with `video/webm;codecs=vp8,opus` and `vp9`. Encoders: FFmpeg's
  libvpx (already built for Splicy export, enable vp8 too) and FFmpeg's native Opus
  encoder; WebM muxer. sex-media `recording` gains a live mode fed from capture.
- `dataavailable` Blobs with timeslice; `canvas.captureStream()` (StreamYard composes
  its scene on a canvas and records/sends that stream).
- Also gives Flash video recording with sound (Andreas asked) by sharing the encoder.

## C4 WebRTC transport
Each piece is a crate with host tests, no_std where it runs in the browser:
- `sex-sdp`: SDP offer/answer parse and write (BUNDLE, rtcp-mux, msid, ssrc, simulcast
  later), transceivers.
- `sex-ice`: ICE agent (host + server-reflexive candidates; STUN binding, consent
  freshness), TURN over UDP and TCP/TLS (StreamYard and most SFUs relay through
  TURN on 443 when UDP is blocked). Needs datagram sockets from sexnet through
  `sex_app::net` (check UDP API; add a UDP Duplex if missing).
- `sex-dtls`: DTLS 1.2 (ECDHE-ECDSA P-256, AES-128-GCM), self-signed certificate,
  fingerprint check, use_srtp extension; reuse sex-tls / sex-crypt primitives.
- `sex-srtp`: SRTP/SRTCP AES-CM-128-HMAC-SHA1-80 and AEAD-AES-128-GCM.
- RTP/RTCP: packetizers for Opus, VP8 (and H.264 for sites that require it), NACK,
  PLI/FIR, REMB/transport-cc bandwidth estimation, jitter buffer for receiving.
- `sex-sctp` (later): data channels.

## C5 RTCPeerConnection API
- `RTCPeerConnection`, `RTCRtpSender/Receiver/Transceiver`, `RTCDataChannel`,
  `RTCIceCandidate`, `RTCSessionDescription`, getStats, replaceTrack, encodings
  (simulcast for SFUs).
- Received tracks play in `<video>`/`<audio>` (decode with sex-media's VP8/VP9/Opus/H.264
  decoders already there).
- Gates: two Kaleidoscope tabs calling each other on loopback; then a host-side
  reference peer (Python aiortc in CI) exchanging media with the browser in QEMU.

## C6 Screen sharing
- `getDisplayMedia`: Shell gives the browser a read-only view of a window or the
  desktop only after the person picks it in Shell's own picker (new Shell capability,
  like the screenshot path). StreamYard uses it for "Share screen".

## C7 Real sites
- StreamYard, Jitsi Meet, Google Meet in browser_sites_gate `--live` runs on the x17
  over the tether (cloud CI only reaches github.com).

## Open items from C1
- Bell asks per program through Spindle (`volume ROW mic on`). A Silkbar consent card
  would be friendlier; the "Add built-in mic support via the audio DSP" thread owns
  Bell's input list and per-app choice, so the mic device list should follow it
  (deviceId per input once Bell lists them).
- Background tabs keep their camera running; Chrome pauses nothing either, but the
  red dot should then move to the tab.
- iframes may not ask (no `allow="camera; microphone"` delegation yet).
