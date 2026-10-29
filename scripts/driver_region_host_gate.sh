#!/usr/bin/env bash
# Host checks of exact driver reservation functions; kernel view lookup is mocked.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-driver-region.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
import pathlib, re, sys
root, out = map(pathlib.Path, sys.argv[1:])
for driver in ('sexusb', 'sexnet'):
    source = (root / 'servers' / driver / 'src/main.rs').read_text()
    functions = re.findall(r'^fn reserve_dma\b.*?^\}', source, re.M | re.S)
    if len(functions) != 1:
        raise SystemExit(f'{driver}: production helper extraction failed')
    (out / f'{driver}.rs').write_text(functions[0])
    if re.search(r'sys_(?:alloc_phys|map_phys)\b|in(?:out)?\("rax"\)\s*(?:30|31|40|50|51)u64', source):
        raise SystemExit(f'{driver}: legacy runtime memory syscall remains')
for path in (root / 'servers/sexfiles/src').rglob('*.rs'):
    if re.search(r'\bsys_(?:grant_mem_lend|map_mem_lend)\s*\(', path.read_text()):
        raise SystemExit(f'{path}: legacy shared-memory call remains')
PY
SEXUSB_DMA_SOURCE="$TEST_DIR/sexusb.rs" SEXNET_DMA_SOURCE="$TEST_DIR/sexnet.rs" \
  rustc --edition=2021 --test "$ROOT_DIR/tests/driver_region.rs" -o "$TEST_DIR/driver-tests"
"$TEST_DIR/driver-tests"
echo '[gate.driver_region_host.result] PASS scope=production_reservation_helpers_mocked_view_lookup'
