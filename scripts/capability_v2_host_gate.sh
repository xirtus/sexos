#!/usr/bin/env bash
# Native tests of authority v2 (plan #405 Phase 3) on the production capability
# table: inline slots, derivation, runtime install, generations, incarnation
# death and torn-read freedom; no kernel/image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-capability-v2.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 -O --test "$ROOT_DIR/tests/capability_v2.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" --nocapture 2>&1 | tee "$TEST_DIR/tests.log" | grep -E '^\[capability\.v2\]|test result' || true
grep -q 'test result: ok' "$TEST_DIR/tests.log" || { cat "$TEST_DIR/tests.log"; echo '[capability.v2.result] FAIL'; exit 1; }
# Compiled controls: a derive that widens rights, and a liveness check that
# ignores generations, must each be caught.
mutant() {
  local name="$1" old="$2" new="$3" test="$4"
  mkdir -p "$TEST_DIR/$name/kernel/src" "$TEST_DIR/$name/tests"
  cp "$ROOT_DIR/kernel/src/capability.rs" "$TEST_DIR/$name/kernel/src/"
  cp "$ROOT_DIR/tests/capability_v2.rs" "$TEST_DIR/$name/tests/"
  python3 - "$ROOT_DIR/kernel/src/capability_table.rs" "$TEST_DIR/$name/kernel/src/capability_table.rs" "$old" "$new" <<'PY'
import sys
source=open(sys.argv[1]).read();old=sys.argv[3]
assert source.count(old)==1, old
open(sys.argv[2],'w').write(source.replace(old,sys.argv[4]))
PY
  rustc --edition=2021 --test "$TEST_DIR/$name/tests/capability_v2.rs" -o "$TEST_DIR/$name/t" 2>/dev/null
  set +e
  "$TEST_DIR/$name/t" --exact "$test" > "$TEST_DIR/$name.log" 2>&1
  local status=$?
  set -e
  if [[ "$status" != 101 ]]; then cat "$TEST_DIR/$name.log"; echo "[capability.v2.negative] FAIL $name"; exit 1; fi
}
mutant widen_rights 'rights & !self.rights != 0' 'false' derivation_narrows_rights_and_keeps_badges
mutant ignore_generation '&& self.object_generation[object].load(Ordering::Acquire) == cap.generation' '' bumping_a_generation_revokes_every_copy_at_once
echo '[capability.v2.negative] PASS widen_rights_rejected=1 ignore_generation_rejected=1'
echo '[capability.v2.result] PASS scope=production_table_inline_slots_derive_install_generations_incarnations_host_threads_not_native_or_cross_core'
