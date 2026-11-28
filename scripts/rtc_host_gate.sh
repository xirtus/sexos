#!/usr/bin/env bash
# WebRTC media transport on the host: DTLS 1.2 with use_srtp (sex-dtls, live against the
# openssl binary when there is one), SRTP/SRTCP known answers (sex-srtp), RTP/RTCP, payload
# formats, jitter buffer and bandwidth estimate (sex-rtp), and a VP8 frame end to end.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
cargo test --locked -p sex-srtp -p sex-rtp -p sex-dtls
