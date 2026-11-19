#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Rebuilds Polyamour's sample Rust .wasm add-on (crates/polyamour-wasm/guests/spiral_rust)
# and checks the build matches the committed crates/polyamour-wasm/tests/spiral_rust.wasm,
# which the host tests load. The build is reproducible, so a mismatch means the guest
# or polyamour-sdk changed without refreshing the binary: run with --update to refresh.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
GUEST=$ROOT/crates/polyamour-wasm/guests/spiral_rust
OUT=$ROOT/target/polyamour-wasm-guest
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
cargo build --locked --release --quiet --manifest-path "$GUEST/Cargo.toml" \
  --target wasm32-unknown-unknown --target-dir "$OUT"
BUILT=$OUT/wasm32-unknown-unknown/release/spiral_rust.wasm
COMMITTED=$ROOT/crates/polyamour-wasm/tests/spiral_rust.wasm
if [[ ${1:-} == --update ]]; then
  cp "$BUILT" "$COMMITTED"; echo "polyamour-wasm-guest: refreshed $(wc -c <"$COMMITTED") bytes"
elif cmp -s "$BUILT" "$COMMITTED"; then
  echo "polyamour-wasm-guest: committed binary matches its source ($(wc -c <"$COMMITTED") bytes)"
else
  echo "polyamour-wasm-guest: tests/spiral_rust.wasm is stale; run scripts/polyamour/check_wasm_guest.sh --update" >&2
  exit 1
fi
