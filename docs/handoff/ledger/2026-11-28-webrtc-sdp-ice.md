# 2026-11-28: SDP, STUN, ICE and TURN for WebRTC (stage C4a of StreamYard)

Goal (Andreas): Kaleidoscope works with StreamYard. This is the networking half of
stage C4 in docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md. DTLS, SRTP and RTP
are a parallel stage (sex-dtls, sex-srtp, sex-rtp); RTCPeerConnection comes later.

## UDP for programs: already there
- No new ABI or op numbers. Programs started with `run +net` already have UDP through
  sexnet: `sex_app::net::UdpSocket` (bind with port 0 for an ephemeral one,
  `local_port`, `send_to`, nonblocking `recv_from`, `recv_from_within`) and the
  lower-level `net::raw::Socket::udp` / `send_to` / `receive_from`, IPv4 and IPv6, on
  `sex_pdx::web` ops UDP_BIND=13, UDP_SEND=14, UDP_RECEIVE=15. The machine's addresses
  (for host candidates) come from `net::addresses` (ADDRESSES=18). The std path
  (`std::net::UdpSocket`) is covered by scripts/program_std_gate.py (stdnet).
- No sexnet change, so no self-wait pin and no new QEMU gate.

## crates/sex-sdp (no_std, no I/O)
- `Session::parse` and `Display` (CRLF): v/o/s/i/u/e/p/c/b/t/r/z/k/a, `m=` sections
  with their c/b/k/a lines. Attributes are kept in order as text, so descriptions write
  back byte for byte.
- Typed attributes (`Attr` trait; `get`, `all`, `push`, `push_flag`, `set_direction`):
  group (BUNDLE), mid, ice-ufrag/pwd/options, fingerprint, setup, rtcp, rtpmap, fmtp,
  rtcp-fb, extmap (with direction), ssrc, ssrc-group, msid, msid-semantic, simulcast
  (RFC 8853, `~` paused, old Firefox `rid=`), rid, sctp-port, max-message-size, ptime,
  candidate (with raddr/rport, tcptype, extensions; trickle `candidate:` strings),
  direction flags. `Media::codecs()` gathers rtpmap/fmtp/rtcp-fb per payload type;
  session-level fallbacks for ICE credentials, fingerprint and setup (Firefox).
- Host test: crates/sex-sdp/tests/browsers.rs with a Chrome offer (audio, simulcast
  video with 25 payload types, data channel, mDNS host candidates), a Firefox offer
  and an ICE lite SFU answer (fixtures in tests/fixtures).

## crates/sex-ice (no_std, sans-IO: datagrams and milliseconds in, datagrams out)
- `stun`: RFC 8489 messages, every class and method, XOR-MAPPED/PEER/RELAYED-ADDRESS
  (v4 and v6), MESSAGE-INTEGRITY (HMAC-SHA1), FINGERPRINT (CRC-32), ERROR-CODE,
  UNKNOWN-ATTRIBUTES, the long-term key (MD5; ASCII credentials, no SASLprep),
  ChannelData and a `Framer` for TURN on TCP/TLS streams. Crypto from RustCrypto
  `hmac` 0.12.1, `sha1` 0.10.7, `md-5` 0.10.6 and `crc32fast` 1.5.4, all already in
  Cargo.lock (sex-wifi, sex-torrent, sex-archive use them). sex-crypt has no SHA-1.
- `turn::Client`: Allocate with 401 realm/nonce, 438 stale nonce, 437 restart,
  Refresh (and lifetime 0 on close), CreatePermission, ChannelBind, Send/Data
  indications, ChannelData; retransmits over UDP, 39.5 s timeout over streams.
  Transport Udp, Tcp or Tls: on a stream the program owns the TCP connection, and for
  TLS calls `tls_start` on it (sexnet's checked TLS, sex-tls), so TURN on 443 works
  through the same bytes.
- `Agent`: one component (BUNDLE + rtcp-mux), host / srflx / relay gathering, trickle
  both ways (`Event::Candidate`, `add_remote_candidate_line`), pairs with RFC 8445
  priorities (IPv6 first), checks paced at 50 ms with retransmissions, triggered
  checks, peer-reflexive remotes, regular nomination (first-check nomination against
  an ICE lite peer), role conflicts (487), consent freshness (RFC 7675: check every
  4-6 s, Disconnected after 10 s, Failed after 30 s), relay-only policy, `send` on the
  selected pair (ChannelData once bound). Non-STUN datagrams come back as
  `Received::Data` for DTLS.
- Host tests: tests/stun.rs (RFC 5769 request and IPv4/IPv6 response vectors), and
  tests/agents.rs on an in-memory network: two agents to connected (v4+v6), data both
  ways, consent kept for a minute and lost after 30 s, a fake STUN server through an
  endpoint-independent NAT, a dead STUN server, a fake TURN server (auth, stale nonce,
  permissions, channels, refresh past 600 s, close) over UDP and TCP, a wrong TURN
  password, a role conflict, an ICE lite peer with late candidates, bad credentials.
- CI: Host regressions job, `cargo test --locked -p sex-sdp -p sex-ice`.

## Left
- mDNS `.local` host candidates from browsers are skipped (SFUs send IPs, so StreamYard
  is fine); ICE-TCP candidates (RFC 6544) are skipped; no ICE restart yet; one
  component only; MESSAGE-INTEGRITY-SHA256 and SASLprep not done.
- The glue: RTCPeerConnection in sex-js, sockets and TURN streams driven by apps/web,
  then DTLS/SRTP on `Received::Data`.
