#!/usr/bin/env bash
# Run the real portable media source and subtitle modules without sex-app's guest allocator.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-media-source.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PYTHON'
import json, sys
from pathlib import Path
src = Path(sys.argv[1]) / 'crates/sex-media/src'
mods = ''.join('#[path = ' + json.dumps(str(src / (m + '.rs')), ensure_ascii=False) + ']\nmod ' + m + ';\n'
               for m in ('source', 'subtitles'))
(Path(sys.argv[2]) / 'test.rs').write_text('extern crate alloc;\n' + mods)
PYTHON
rustc --edition=2021 --test "$TEST_DIR/test.rs" -o "$TEST_DIR/media-source-tests"
"$TEST_DIR/media-source-tests"
