# 2026-11-28: WebRTC security and media transport (DTLS-SRTP, SRTP, RTP/RTCP), stage C4b of StreamYard

Goal (Andreas): Kaleidoscope works with StreamYard. Plan: docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md.
This stage is the sans-IO, no_std + alloc half of C4. UDP, SDP and ICE are a separate stage.
Stacked on stage C1 (PR #863).

## crates/sex-dtls (DTLS 1.2, RFC 6347)
- Client and server. TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256, P-256, extended master secret
  required, use_srtp (RFC 5764) with SRTP_AEAD_AES_128_GCM and SRTP_AES128_CM_HMAC_SHA1_80.
- Self-signed P-256 certificate (cert.rs), sha-256 fingerprint check against SDP's
  `a=fingerprint` (constant time), HelloVerifyRequest cookie, flights retransmitted with
  backoff, handshake fragmentation to the MTU and reassembly in any order, 64-record replay
  window, close_notify, keying material export (`EXTRACTOR-dtls_srtp`) into sex-srtp contexts.
- Same RustCrypto primitives and pinned versions as sex-tls / sex-crypt.

## crates/sex-srtp (RFC 3711, RFC 7714)
- SRTP and SRTCP for AES-CM + HMAC-SHA1-80 and AEAD AES-128-GCM. Key derivation, ROC
  tracking, 64-packet replay windows, constant-time tag compares. Known answers from
  libsrtp's srtp_validate and srtp_validate_gcm (RFC 3711 KDF), ROC rollover and reorder.

## crates/sex-rtp (RFC 3550 and friends)
- RTP with CSRC, padding, one- and two-byte header extensions, abs-send-time.
- RTCP SR, RR, SDES, BYE, generic NACK, PLI, FIR, REMB, transport-cc; compound packets.
- Payload formats: Opus, VP8, VP9 (basic), H.264 single NAL, STAP-A, FU-A.
- Jitter buffer that assembles frames and asks for NACKs; GCC-like bandwidth estimate
  (delay gradient trendline plus loss).

## Tests
- `bash scripts/rtc_host_gate.sh` (CI Host regressions job). It includes a DTLS handshake in
  memory, SRTP keys from it, and a VP8 frame through RTP + SRTP intact (sex-dtls/tests/media.rs).
- sex-dtls/tests/openssl.rs: live handshakes against the `openssl` binary in both roles, keying
  material compared. Skipped without OpenSSL 1.1.1+; the gate makes it required on Linux.

## Left
- UDP, SDP, ICE (other stage), RTCPeerConnection in sex-js, codecs themselves.
