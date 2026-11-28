#!/usr/bin/env bash
# WebRTC media transport on the host: DTLS 1.2 with use_srtp (sex-dtls, live against the
# openssl binary when there is one), SRTP/SRTCP known answers (sex-srtp), RTP/RTCP, payload
# formats, jitter buffer and bandwidth estimate (sex-rtp), and a VP8 frame end to end.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
# Linux runners ship OpenSSL: there the live interop tests must run, not skip.
if [[ "$(uname -s)" == Linux ]]; then
  command -v openssl >/dev/null || { echo "rtc_host_gate: openssl binary missing" >&2; exit 1; }
  export SEX_DTLS_REQUIRE_OPENSSL=1
fi
command -v openssl >/dev/null && openssl version || true
cargo test --locked -p sex-srtp -p sex-rtp -p sex-dtls
