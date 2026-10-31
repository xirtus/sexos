#!/usr/bin/env bash
# Native tests of the production capability table's sealed revocation and the
# terminal-incarnation revocation wiring; no kernel/image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-capability-revocation.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/capability_revocation.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
# Compiled control: a revoke_all that only counts must leave authority held.
mkdir -p "$TEST_DIR/mutant/kernel/src" "$TEST_DIR/mutant/tests"
cp "$ROOT_DIR/kernel/src/capability.rs" "$ROOT_DIR/kernel/src/app_resources.rs" "$TEST_DIR/mutant/kernel/src/"
cp "$ROOT_DIR/tests/capability_revocation.rs" "$TEST_DIR/mutant/tests/"
python3 - "$ROOT_DIR/kernel/src/capability_table.rs" "$TEST_DIR/mutant/kernel/src/capability_table.rs" <<'PY'
import sys
source=open(sys.argv[1]).read();old='self.revoke(id).is_some()'
assert source.count(old)==1
open(sys.argv[2],'w').write(source.replace(old,'self.find(id).is_some()'))
PY
rustc --edition=2021 --test "$TEST_DIR/mutant/tests/capability_revocation.rs" -o "$TEST_DIR/mutant-tests"
set +e
"$TEST_DIR/mutant-tests" --exact revocation_never_creates_authority > "$TEST_DIR/mutant.log" 2>&1
MUTANT_STATUS=$?
set -e
if [[ "$MUTANT_STATUS" != 101 ]]; then cat "$TEST_DIR/mutant.log"; echo '[capability.revocation.negative] FAIL'; exit 1; fi
echo '[capability.revocation.negative] PASS counting_revoke_all_rejected=1'
echo '[capability.revocation.result] PASS scope=production_table_revocation_and_terminal_wiring_source_not_native_or_cross_core'
