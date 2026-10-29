#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-service-custody.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
python3 - "$TEST_DIR" <<'PY'
from pathlib import Path
import sys
s=Path('servers/sexfiles/src/backends/mod.rs').read_text()
Path(sys.argv[1],'backend_trait.rs').write_text(s[:s.index('pub mod ramfs;')])
lib=Path('crates/sex-pdx/src/lib.rs').read_text()
line=next(line for line in lib.splitlines() if line.startswith('pub const DISKFS_V2_MAX_READ:'))
Path(sys.argv[1],'diskfs_constant.rs').write_text(line+'\n')
PY
rustc --edition=2021 --test tests/service_custody.rs -o "$TEST_DIR/custody"
"$TEST_DIR/custody"
APP_RESOURCE_TEST_DIR="$TEST_DIR" rustc --edition=2021 --test tests/ramfs_retirement.rs -o "$TEST_DIR/ramfs"
"$TEST_DIR/ramfs"
echo '[service.custody.host.gate] PASS scope=production_service_custody_ramfs_with_mock_authority_and_host_locks'
