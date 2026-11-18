#!/usr/bin/env bash
# Native tests of the IPC v2 objects (endpoint, one-shot reply, notification) and
# the sex-pdx message layout, plus compiled controls that must fail; no image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
T="$(mktemp -d "${TMPDIR:-/tmp}/sexos-ipc-v2.XXXXXX")"
trap 'rm -rf "$T"' EXIT
for t in ipc_v2_endpoint ipc_v2_reply ipc_v2_notification sex_pdx_endpoint; do
  rustc --edition=2021 -O --test "$ROOT_DIR/tests/$t.rs" -o "$T/$t"
  "$T/$t" -q
done
# Controls: each mutant breaks one guarantee; its named test must fail.
mutant() { # file old new test
  rm -rf "$T/m"; mkdir -p "$T/m/kernel/src/ipc_v2" "$T/m/tests"
  cp "$ROOT_DIR"/kernel/src/ipc_v2/*.rs "$T/m/kernel/src/ipc_v2/"
  cp "$ROOT_DIR"/tests/ipc_v2_*.rs "$T/m/tests/"
  python3 - "$T/m/kernel/src/ipc_v2/$1" "$2" "$3" <<'PY'
import sys
p,old,new=sys.argv[1:];s=open(p).read();assert s.count(old)==1,old
open(p,'w').write(s.replace(old,new))
PY
  local test_file; case "$1" in endpoint.rs) test_file=ipc_v2_endpoint;; reply.rs) test_file=ipc_v2_reply;; *) test_file=ipc_v2_notification;; esac
  rustc --edition=2021 --test "$T/m/tests/$test_file.rs" -o "$T/mutant"
  set +e; timeout 60 "$T/mutant" --exact "$4" >"$T/mutant.log" 2>&1; local st=$?; set -e
  if [[ $st == 0 ]]; then cat "$T/mutant.log"; echo "[ipc_v2.negative] FAIL $4 passed with mutant"; exit 1; fi
}
mutant endpoint.rs 'if waiter != NO_WAITER {' 'if false {' one_receiver_at_a_time
mutant reply.rs 'if generations.is_current(cap.caller, cap.generation) { Ok(cap) }' 'if true { Ok(cap) }' caller_death_cancels_the_outstanding_reply
mutant notification.rs 'w => SignalOutcome::Wake(w - 1),' '_ => SignalOutcome::Pending,' signal_wakes_the_parked_waiter
mutant endpoint.rs 'queue.len -= 1;
        Some(removed)' 'Some(removed)' cancelled_caller_leaves_the_others_in_order
echo '[ipc_v2.negative] PASS mutants_rejected=4'
echo '[ipc_v2.result] PASS scope=endpoint_reply_notification_state_machines_host'
