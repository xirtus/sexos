#!/usr/bin/env bash
# Host checks of exact driver reservation functions; kernel view lookup is mocked.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-driver-region.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
import pathlib, re, sys
root, out = map(pathlib.Path, sys.argv[1:])
# sexnet keeps its reservation helper in main.rs; it is extracted and run below.
source = (root / 'servers/sexnet/src/main.rs').read_text()
functions = re.findall(r'^fn reserve_dma\b.*?^\}', source, re.M | re.S)
if len(functions) != 1:
    raise SystemExit('sexnet: production helper extraction failed')
(out / 'sexnet.rs').write_text(functions[0])
# sexusb (xHCI rewrite) builds one DmaRegionPool from the DMA region view in main.rs and
# hands it to xhci::page; its runtime use is proven by usb_path_gate. Here: the pool must
# come from the full DMA view, and no legacy memory syscall may remain in any USB source.
usb_main = (root / 'servers/sexusb/src/main.rs').read_text()
if not re.search(r'memory_region_full_view\(\s*sex_pdx::SLOT_DMA_REGION,\s*QUERY_VIEW \| READ \| WRITE \| DMA_ADDRESS\)', usb_main) \
        or 'DmaRegionPool::new(view)' not in usb_main:
    raise SystemExit('sexusb: DMA pool is not built from the leased DMA region view')
if not re.search(r'pool\.reserve\(pages \* PAGE\)', (root / 'servers/sexusb/src/xhci.rs').read_text()):
    raise SystemExit('sexusb: xhci::page no longer reserves from the region pool')
for driver, source in [('sexnet', source)] + [(f'sexusb/{p.name}', p.read_text())
                                              for p in sorted((root / 'servers/sexusb/src').glob('*.rs'))]:
    if re.search(r'sys_(?:alloc_phys|map_phys)\b|in(?:out)?\("rax"\)\s*(?:30|31|40|50|51)u64', source):
        raise SystemExit(f'{driver}: legacy runtime memory syscall remains')
for path in (root / 'servers/sexfiles/src').rglob('*.rs'):
    if re.search(r'\bsys_(?:grant_mem_lend|map_mem_lend)\s*\(', path.read_text()):
        raise SystemExit(f'{path}: legacy shared-memory call remains')
PY
SEXNET_DMA_SOURCE="$TEST_DIR/sexnet.rs" \
  rustc --edition=2021 --test "$ROOT_DIR/tests/driver_region.rs" -o "$TEST_DIR/driver-tests"
"$TEST_DIR/driver-tests"
echo '[gate.driver_region_host.result] PASS scope=production_reservation_helpers_mocked_view_lookup'
