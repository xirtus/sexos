#!/usr/bin/env bash
# Qupid's pure-Rust media intelligence on the host: episode/binge matching, skip and
# "Up next" cards, 50%-300% projector audio DSP + 3-band EQ, non-truncated stream
# telemetry + season-pack ranking, and ProviderPack hydration (crates/sex-episodes).
set -euo pipefail
cd "$(dirname "$0")/.."
TEST_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_DIR"' EXIT
CFG_ARGS=(--check-cfg 'cfg(test)' --check-cfg 'cfg(qupid_seed_pack)')
if [[ -f crates/sex-episodes/qupid_seed_pack.json ]]; then
    CFG_ARGS+=(--cfg qupid_seed_pack)
fi
rustc --edition=2021 "${CFG_ARGS[@]}" -A dead_code --test tests/qupid_binge.rs -o "$TEST_DIR/qupid-binge"
"$TEST_DIR/qupid-binge"
