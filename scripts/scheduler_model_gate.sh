#!/usr/bin/env bash
# Kernel pass 2, K6: the scheduler custody model (tests/model_scheduler_wait.rs) over
# the real kernel/src/task_ownership.rs and self_wait.rs, plus compiled controls: each
# injected bug in those files must fail the named model test. Host-only, no image.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-scheduler-model.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 -O --test "$ROOT_DIR/tests/model_scheduler_wait.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests" 2>&1 | tee "$TEST_DIR/tests.log" | grep -E '^test |test result' || true
grep -q 'test result: ok' "$TEST_DIR/tests.log" || { cat "$TEST_DIR/tests.log"; echo '[scheduler.model.result] FAIL'; exit 1; }
mutant() {
  local name="$1" file="$2" old="$3" new="$4" test="$5"
  mkdir -p "$TEST_DIR/$name/kernel/src" "$TEST_DIR/$name/tests"
  cp "$ROOT_DIR/kernel/src/task_ownership.rs" "$ROOT_DIR/kernel/src/self_wait.rs" "$TEST_DIR/$name/kernel/src/"
  cp "$ROOT_DIR/tests/model_scheduler_wait.rs" "$TEST_DIR/$name/tests/"
  python3 - "$ROOT_DIR/kernel/src/$file" "$TEST_DIR/$name/kernel/src/$file" "$old" "$new" <<'PY'
import sys
source=open(sys.argv[1]).read();old=sys.argv[3]
assert source.count(old)==1, 'mutation site missing or ambiguous: '+old
open(sys.argv[2],'w').write(source.replace(old,sys.argv[4]))
PY
  rustc --edition=2021 -O --test "$TEST_DIR/$name/tests/model_scheduler_wait.rs" -o "$TEST_DIR/$name/t" 2>"$TEST_DIR/$name.compile.log" \
    || { cat "$TEST_DIR/$name.compile.log"; echo "[scheduler.model.negative] FAIL $name did not compile"; exit 1; }
  set +e
  "$TEST_DIR/$name/t" --exact "$test" > "$TEST_DIR/$name.log" 2>&1
  local status=$?
  set -e
  if [[ "$status" != 101 ]] || ! grep -q "test $test ... FAILED" "$TEST_DIR/$name.log"; then
    cat "$TEST_DIR/$name.log"; echo "[scheduler.model.negative] FAIL $name escaped $test"; exit 1
  fi
  echo "[scheduler.model.negative] $name caught by $test"
}
mutant park_ignores_pending task_ownership.rs 'let pending = old & PENDING != 0;' 'let pending = false;' park_consumes_exactly_the_pending_wake
mutant wake_lost_on_switch_out task_ownership.rs 'let new = old | PENDING;' 'let new = if phase(old) == Phase::SwitchingOut { old } else { old | PENDING };' a_wake_is_never_lost_from_any_state
mutant wake_after_exit task_ownership.rs 'if old & EXITED != 0 || phase(old) == Phase::Dead { return WakeResult::Exited; }' 'if phase(old) == Phase::Dead { return WakeResult::Exited; }' every_reachable_custody_state_keeps_the_invariants
mutant admit_keeps_block task_ownership.rs 'if old & BLOCKED != 0 { new &= !(BLOCKED | PENDING); }' '' every_reachable_custody_state_keeps_the_invariants
mutant second_token task_ownership.rs 'if phase(old) != Phase::Detached || !runnable(old) { return Admission::NotRunnable; }' 'if !matches!(phase(old), Phase::Detached | Phase::Queued) || !runnable(old) { return Admission::NotRunnable; }' every_reachable_custody_state_keeps_the_invariants
mutant dispatch_dead task_ownership.rs 'let exited = old & EXITED != 0;
            let new = if exited { with_phase(old, Phase::Dead) }
                else { with_phase(old, Phase::Dispatching)' 'let exited = false;
            let new = if exited { with_phase(old, Phase::Dead) }
                else { with_phase(old, Phase::Dispatching)' an_exit_from_any_state_ends_in_dead_without_dispatch
mutant fault_woken task_ownership.rs 'word & (EXITED | FAULT_WAIT) == 0 &&' 'word & EXITED == 0 &&' a_fault_wait_holds_until_the_pager_completes_it
mutant deadline_early self_wait.rs 'if at==0||now<at{return None;}' 'if at==0{return None;}' deadlines_match_the_reference_over_every_short_sequence
mutant deadline_rearm self_wait.rs 'self.at.compare_exchange(0,at,' 'self.at.swap(at,Ordering::AcqRel);self.at.compare_exchange(at,at,' deadlines_match_the_reference_over_every_short_sequence
mutant inventory_after_seal self_wait.rs 'if self.sealed.load(Ordering::Acquire)||index>=N' 'if index>=N' boot_inventory_matches_the_reference_over_every_short_sequence
echo '[scheduler.model.result] PASS mutants_rejected=10 scope=reachable_custody_graph_two_cores_deadlines_inventory_real_sources_not_x86_stack_or_interleavings'
