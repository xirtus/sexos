#!/usr/bin/env bash
# Native tests of actual kernel selector decoding; no kernel/image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-authority-slots.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/authority_slot_validation.rs -o "$TEST_DIR/authority-tests"
"$TEST_DIR/authority-tests"
# Compiled control: restoring the retired ambient framebuffer disclosure must fail.
mkdir -p "$TEST_DIR/mutant/kernel/src/syscalls" "$TEST_DIR/mutant/tests"
cp kernel/src/syscalls/mod.rs "$TEST_DIR/mutant/kernel/src/syscalls/"
cp kernel/src/ipc.rs kernel/src/capability.rs "$TEST_DIR/mutant/kernel/src/"
python3 - "$ROOT_DIR/kernel/src/capability_slot.rs" "$TEST_DIR/mutant/kernel/src/capability_slot.rs" <<'PY'
import sys
source=open(sys.argv[1]).read();old='        0x12 => Some(BridgeOp::InitArg),\n'
assert source.count(old)==1
open(sys.argv[2],'w').write(source.replace(old,old+'        0xE3 => Some(BridgeOp::BootDisplayQuery),\n'))
PY
cp tests/authority_slot_validation.rs "$TEST_DIR/mutant/tests/"
rustc --edition=2021 --test "$TEST_DIR/mutant/tests/authority_slot_validation.rs" -o "$TEST_DIR/mutant-tests"
for MUTANT_TEST in slot_zero_bridge_admits_only_caller_scoped_operations retired_ambient_display_disclosure_and_present_hook_deny; do
  set +e
  "$TEST_DIR/mutant-tests" --exact "$MUTANT_TEST" > "$TEST_DIR/$MUTANT_TEST.log" 2>&1
  MUTANT_STATUS=$?
  set -e
  if [[ "$MUTANT_STATUS" != 101 ]]; then cat "$TEST_DIR/$MUTANT_TEST.log"; echo '[authority.slot_validation.negative] FAIL'; exit 1; fi
done
echo '[authority.slot_validation.negative] PASS restored_0xE3_rejected=2'
echo '[authority.slot_validation.gate.result] PASS'
