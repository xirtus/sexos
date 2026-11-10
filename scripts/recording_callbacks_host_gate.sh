#!/usr/bin/env bash
# Actual Rust callback state; scripted C ABI, independently mutation-checked.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-record-callbacks.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test --cfg ffmpeg_recording -A dead_code \
  "$ROOT_DIR/tests/recording_callbacks.rs" -o "$TEST_DIR/positive"
"$TEST_DIR/positive" --test-threads=1
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import shutil, subprocess, sys
root, out = map(Path, sys.argv[1:])
tree = out / 'ignored-seek'
for name in ['tests/recording_callbacks.rs', 'crates/sex-media/src/recording/mod.rs', 'crates/sex-media/src/recording/output.rs']:
    target = tree / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / name, target)
source = tree / 'crates/sex-media/src/recording/mod.rs'
text = source.read_text()
old = 'writer.failure = Some(error.into());'
assert text.count(old) == 1, 'mutation binding'
source.write_text(text.replace(old, 'let _ = error;', 1))
binary = tree / 'test'
subprocess.run(['rustc', '--edition=2021', '--test', '--cfg', 'ffmpeg_recording', '-A', 'warnings', str(tree / 'tests/recording_callbacks.rs'), '-o', str(binary)], check=True)
run = subprocess.run([str(binary), '--test-threads=1'], capture_output=True, text=True)
assert run.returncode == 101 and 'ignored_seek_failure_is_terminal_and_never_reports_end ... FAILED' in run.stdout, run.stdout + run.stderr
print('[recording.callback.negative] rejected=ignored_seek scripted_c_abi=1')
PY
