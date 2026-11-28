# 2026-11-28: RTCPeerConnection in Kaleidoscope, stage C5 of StreamYard

Goal (Andreas): Kaleidoscope works with StreamYard. Plan:
docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md. Builds on C1 (getUserMedia, #863),
sex-sdp and sex-ice (#867), sex-dtls, sex-srtp and sex-rtp (#868).

## Engine (new crate crates/sex-rtc, no_std + alloc, sans-IO)
- `Peer`: JSEP offer/answer with BUNDLE, rtcp-mux, mids, transceivers (addTrack reuses
  a free recvonly one), directions and currentDirection, stopped m-lines, codec
  preferences, extmaps (abs-send-time, transport-cc, sdes:mid). Codecs offered: opus 111,
  PCMU 0, PCMA 8, VP8 96, VP9 98, H264 102. Answers rank what we can encode first.
- Transport: sex-ice agent (host, srflx, relay over UDP), sex-dtls client or server
  by a=setup, SRTP keys from DTLS. Candidates trickle out as events and go into
  localDescription.
- Media: RTP routed by SSRC, then mid header extension, then payload type. Jitter buffer
  and VP8 depacketizer from sex-rtp. NACK is answered by resend, PLI/FIR by a key frame,
  REMB and TWCC feed the bandwidth estimate, SR/RR give RTT. We send NACK and PLI every
  20 ms, TWCC every 100 ms, SR/RR/SDES every second.
- `Rtc` (host.rs): the page protocol behind op::RTC. Peers belong to a page (owner) and
  go when it leaves. Text verbs: new, offer, answer, local, remote, addtrack,
  addtransceiver, removetrack, direction, stop, prefs, replace, params, candidate,
  close, restart, desc, stats, state. Errors are "!Name\tmessage" so pages get the right
  DOMException.
- VP8 encoder (vp8.rs, RFC 6386): key frames with DC prediction, inter frames as
  skipped ZEROMV or intra blocks, no loop filter, rate control on q. Tables from
  image-webp 0.2.4 (MIT OR Apache-2.0). Host test: a 24 frame moving sequence is decoded
  bit for bit equal by FFmpeg's vp8 and libvpx decoders.
- G.711 mu-law and A-law with 48 kHz to 8 kHz resampling (g711.rs).

## Pages (sex-js)
- New text op `RTC = 42`. Document field `rtc` is a hook set by the browser.
- RTCPeerConnection (createOffer, createAnswer, set/LocalDescription (implicit too),
  setRemoteDescription, addIceCandidate, addTrack, removeTrack, addTransceiver,
  getSenders/getReceivers/getTransceivers, getStats, restartIce, close,
  getConfiguration/setConfiguration, generateCertificate, all events and legacy
  callbacks), RTCSessionDescription, RTCIceCandidate, RTCRtpSender (replaceTrack,
  get/setParameters, getCapabilities), RTCRtpReceiver, RTCRtpTransceiver
  (setCodecPreferences, direction, stop), RTCStatsReport, RTCCertificate,
  RTCDataChannel, event classes, webkitRTCPeerConnection.
- Remote tracks play through `video.srcObject` (live source `rtc:ID:N`).
- Host test: crates/sex-js/tests/rtc.rs (two peers in one page reach connected, unmute,
  stats, close).

## Browser (apps/web)
- apps/web/src/rtc.rs: UDP sockets per peer (sexnet), ICE server names resolved,
  timers, sends the camera as VP8 (15 fps, scaleResolutionDownBy, halves again under
  300 kbps) and the mic as PCMU/PCMA, decodes remote VP8/VP9/H264/Opus with
  sex-media's new `frames::FrameDecoder` (FFmpeg C bridge, no container) and G.711
  itself, plays remote sound, shows remote pictures in elements.
- media.rs "live" verb takes "PICTURE [SOUND]"; capture.rs keeps heard mic samples.
- Gate: scripts/program_web_webrtc_gate.py with --fake-media and
  scripts/web_site/webrtc.html (two peers over loopback UDP, remote video moves,
  stats rise). In CI next to the capture gate. Host tests: `cargo test -p sex-rtc`
  (CI step after the FFmpeg install, SEX_RTC_REQUIRE_FFMPEG=1).

## Not done
- Data channels: no SCTP. RTCDataChannel stays "connecting" and never opens.
- Opus send: the CI FFmpeg has no Opus or libvpx encoder, so the mic goes as G.711
  and the camera through our own VP8 encoder. Opus is received. With an FFmpeg that
  has libvpx/libopus, wire them in under cfg(ffmpeg).
- Overlap: the recorder's sex-media live encoder makes muxed WebM only, so it was not
  reused for RTP frames. One of the two should own VP8 encoding later.
- Simulcast: rids are accepted and reported, but one layer is sent.
- restartIce is a stub: it keeps the ICE credentials and does not renegotiate.
- TURN over TCP/TLS is parsed but the stream sockets are not wired. No mDNS candidates.
- No screen share track yet (getDisplayMedia is a later stage). Tap sources (Web Audio)
  are accepted as a source name but not sent.
- Seeds come from rdrand plus rdtsc in the browser, not from a system entropy service.
- The optional aiortc interop gate was skipped (no aiortc in the CI image).
