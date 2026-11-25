#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Host gate for DNS cache + DoT/DoH, TLS 1.3 session resumption, HTTP/2, and HTTP/3/QUIC.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== [1/5] sex-net host tests (DNS cache + DoT/DoH + IPv4/IPv6 stack) ==="
cargo test -p sex-net --lib

echo "=== [2/5] sex-http host tests (HTTP/1.x + HTTP/2/HPACK + HTTP/3/QUIC/QPACK/Alt-Svc) ==="
cargo test -p sex-http --lib

echo "=== [3/5] sex-tls host tests (ResumptionStore LRU + TLS 1.3 session resumption + keep-alive) ==="
cargo test -p sex-tls --features open-web --lib -- \
  resumption_store_lru_evicts_oldest_entries \
  second_connection_to_same_host_resumes_tls_session \
  one_session_carries_several_requests

echo "=== [4/5] TLS platform & scalar crypto verification ==="
python3 scripts/check_tls_platform.py

echo "=== [5/5] Architectural truth verification ==="
python3 scripts/check_architectural_truth.py

echo "=== PASS: web_protocols_host_gate ==="
