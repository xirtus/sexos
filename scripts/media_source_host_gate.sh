#!/usr/bin/env bash
# Run the real portable media source module without sex-app's guest allocator.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-media-source.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PYTHON'
import json, sys
from pathlib import Path
source = Path(sys.argv[1]) / 'crates/sex-media/src/source.rs'
(Path(sys.argv[2]) / 'test.rs').write_text('extern crate alloc;\n#[path = '
    + json.dumps(str(source), ensure_ascii=False) + ']\nmod source;\n')
PYTHON
rustc --edition=2021 --test "$TEST_DIR/test.rs" -o "$TEST_DIR/media-source-tests"
"$TEST_DIR/media-source-tests"
