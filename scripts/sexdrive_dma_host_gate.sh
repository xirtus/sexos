#!/usr/bin/env bash
# Production userland DMA lifetime and extracted driver peer-check tests.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexdrive-dma.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/block_view.rs" "$TEST_DIR/read_path.rs" <<'PY'
from pathlib import Path
import sys
source = (Path(sys.argv[1]) / 'apps/sexdrive/src/main.rs').read_text()
start = source.index('fn block_buffer_view(')
end = source.index('\nfn ', start + 1)
Path(sys.argv[2]).write_text(source[start:end])
parts = []
for name in ['stop_uncertain_io', 'prepare_bounce', 'nvme_read_into_mapped_va', 'nvme_write_one_block']:
    start = source.index('fn ' + name + '(')
    end = source.index('\nfn ', start + 1)
    parts.append(source[start:end])
Path(sys.argv[3]).write_text('\n'.join(parts))
PY
SEXDRIVE_BLOCK_VIEW_SOURCE="$TEST_DIR/block_view.rs" SEXDRIVE_READ_PATH_SOURCE="$TEST_DIR/read_path.rs" rustc --edition=2021 --test \
  "$ROOT_DIR/tests/sexdrive_dma_transfer.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" --nocapture
echo '[gate.sexdrive_dma_host.result] PASS scope=production_lifetime_peer_check_not_device_DMA_containment'
