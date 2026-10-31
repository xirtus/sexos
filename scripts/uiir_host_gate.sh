#!/usr/bin/env bash
# First UiIR stream: production ring/cursor protocol over host pages, BOOT
# grant shape and attach checks. Not epoch replay or a rendering consumer.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-uiir.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 -O --test "$ROOT_DIR/tests/uiir.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
# Compiled control: a consumer that trusts record sequences must fail.
mkdir -p "$TEST_DIR/mutant/crates/sex-pdx/src" "$TEST_DIR/mutant/kernel/src" "$TEST_DIR/mutant/tests"
cp "$ROOT_DIR/tests/uiir.rs" "$TEST_DIR/mutant/tests/"
cp "$ROOT_DIR/kernel/src/boot_memory.rs" "$ROOT_DIR/kernel/src/init.rs" "$TEST_DIR/mutant/kernel/src/"
cp "$ROOT_DIR/crates/sex-pdx/src/lib.rs" "$TEST_DIR/mutant/crates/sex-pdx/src/"
python3 - "$ROOT_DIR/crates/sex-pdx/src/uiir.rs" "$TEST_DIR/mutant/crates/sex-pdx/src/uiir.rs" <<'PY'
import sys
source=open(sys.argv[1]).read();old='        if record.sequence != self.expected { return Err(Error::Order { expected: self.expected, got: record.sequence }); }\n'
assert source.count(old)==1
open(sys.argv[2],'w').write(source.replace(old,''))
PY
rustc --edition=2021 --test "$TEST_DIR/mutant/tests/uiir.rs" -o "$TEST_DIR/mutant-tests"
set +e
"$TEST_DIR/mutant-tests" --exact consumer_rejects_a_record_that_breaks_the_stream > "$TEST_DIR/mutant.log" 2>&1
STATUS=$?
set -e
if [[ "$STATUS" != 101 ]]; then cat "$TEST_DIR/mutant.log"; echo '[uiir.negative] FAIL'; exit 1; fi
echo '[uiir.negative] PASS unchecked_sequence_rejected=1'
echo '[uiir.result] PASS scope=spsc_ring_cursor_protocol_boot_grant_shape_not_epoch_replay_or_rendering'
