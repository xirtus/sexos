#!/usr/bin/env bash
# Host verification gate for SexOS modern TCP transport & zero-copy datapath:
# - RFC 7323 Window Scaling
# - RFC 2018 / 6675 SACK codec, receiver scoreboard, and sender fast recovery
# - RFC 9438 CUBIC (+ HyStart++) and BBR integer/fixed-point congestion control
# - Lock-free SPSC zero-copy descriptor rings, SgSlice, Toeplitz/FNV RSS, PerCoreLanes
# - Multi-frame Adapter burst batching + full regression check via sexnet_transport_host_gate.sh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 1. Verify no floating-point types appear in the new no_std transport modules.
python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import re, sys

root = Path(sys.argv[1])
for rel in (
    "crates/sex-net/src/tcp_options.rs",
    "crates/sex-net/src/congestion.rs",
    "crates/sex-net/src/datapath.rs",
):
    text = (root / rel).read_text()
    # Strip line comments before checking for f32/f64 tokens.
    code = "\n".join(line.split("//", 1)[0] for line in text.splitlines())
    m = re.search(r"\b(f32|f64)\b", code)
    assert m is None, f"Unexpected floating-point type {m.group(0)} in {rel}"
print("[tcp.modern.no_fpu] PASS modules=tcp_options,congestion,datapath fpu=0")
PY

# 2. Run the full production-locked transport host gate (includes all existing + new unit tests).
bash "$ROOT_DIR/scripts/sexnet_transport_host_gate.sh"

# 3. Run targeted filter verification to guarantee every modern transport test suite executes.
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tcp-modern-gate.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
PACKAGE_DIR="$TEST_DIR/crates/sex-net"
python3 - "$ROOT_DIR" "$PACKAGE_DIR" <<'PY'
from pathlib import Path
import shutil, sys
root, out = map(Path, sys.argv[1:])
out.mkdir(parents=True)
shutil.copytree(root / "crates/sex-net/src", out / "src")
shutil.copyfile(root / "Cargo.lock", out / "Cargo.lock")
shutil.copytree(root / "crates/sex-net-config", out.parent / "sex-net-config")
(out / "Cargo.toml").write_text(
    (root / "crates/sex-net/Cargo.toml").read_text()
    + "\n[workspace]\n\n[patch.crates-io]\n"
    + f'smoltcp = {{ path = "{root / "vendor/smoltcp-0.14.0"}" }}\n'
)
PY

cargo metadata --manifest-path "$PACKAGE_DIR/Cargo.toml" --offline --format-version 1 > /dev/null
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
for filter in "tcp_options::tests" "congestion::tests" "datapath::tests" "adapter_multi_frame_burst_batching"; do
    CARGO_TARGET_DIR="$TEST_DIR/target" cargo test \
        --manifest-path "$PACKAGE_DIR/Cargo.toml" \
        --target "$HOST_TARGET" \
        --locked --offline --lib "$filter"
done

echo "[tcp.modern.host] PASS window_scale=rfc7323 sack=rfc2018_6675 cc=cubic_bbr_nofpu datapath=spsc_rss_per_core_lanes adapter=burst_ring"
