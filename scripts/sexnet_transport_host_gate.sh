#!/usr/bin/env bash
# Actual protocol engine + raw independent peer. No NIC or authority proof.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexnet-transport.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import shutil,sys
root,out=map(Path,sys.argv[1:]);shutil.copytree(root/'crates/sex-net/src',out/'src');shutil.copyfile(root/'Cargo.lock',out/'Cargo.lock')
# Only isolate workspace discovery; production package/dependencies/source intact.
(out/'Cargo.toml').write_text((root/'crates/sex-net/Cargo.toml').read_text()+'\n[workspace]\n')
PY
cargo metadata --manifest-path "$TEST_DIR/Cargo.toml" --offline --format-version 1 > "$TEST_DIR/metadata.json"
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import json,sys,tomllib,hashlib
root,out=map(Path,sys.argv[1:]);production=tomllib.loads((root/'Cargo.lock').read_text());isolated=tomllib.loads((out/'Cargo.lock').read_text())
expected={(p['name'],p['version']):(p.get('source'),p.get('checksum'))for p in production['package']}
for package in isolated['package']:
 key=package['name'],package['version'];assert key in expected,key
 assert expected[key]==(package.get('source'),package.get('checksum')),key
for p in(root/'crates/sex-net/src').rglob('*.rs'):
 assert p.read_bytes()==(out/'src'/p.relative_to(root/'crates/sex-net/src')).read_bytes(),p
meta=json.loads((out/'metadata.json').read_text());packages={p['id']:p for p in meta['packages']}
stack=[n for n in meta['resolve']['nodes']if packages[n['id']]['name']=='smoltcp'];assert len(stack)==1
assert packages[stack[0]['id']]['version']=='0.14.0'
assert set(stack[0]['features'])=={'medium-ethernet','proto-ipv4','socket','socket-tcp','socket-udp'},stack[0]['features']
assert not any(p['name']=='libc'for p in meta['packages'])
print('[sexnet.transport.dependencies] exact_production_lock_versions_checksums=1 smoltcp=0.14.0 std=0 alloc=0 host_adapters=0')
PY
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
CARGO_TARGET_DIR="$TEST_DIR/target" cargo test --manifest-path "$TEST_DIR/Cargo.toml" --target "$HOST_TARGET" --locked --offline --lib
# The dependency check and test compilation never update the production lock.
echo '[sexnet.transport.host] PASS scope=actual_protocol_engine_with_raw_frame_clock_peer_not_live_nic_authority_or_hostile_containment'
