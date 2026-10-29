#!/usr/bin/env bash
# Exercise declared stage cfgs through the sealed build entrypoint, then restore
# and verify the normal desktop image. Never reuse a profile's ISO as normal.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
PROOF_DIR="${GATE_DIR:-/tmp/sexos_cleanup_profiles}"
mkdir -p "$PROOF_DIR"
SPEC_BACKUP="$(mktemp "${TMPDIR:-/tmp}/sexos-cleanup-spec.XXXXXX")"
cp sexos_build_spec.toml "$SPEC_BACKUP"
ISO_BACKUP=""
if [[ -f sexos-v1.0.0.iso ]]; then
  ISO_BACKUP="$(mktemp "${TMPDIR:-/tmp}/sexos-cleanup-iso.XXXXXX")"
  cp sexos-v1.0.0.iso "$ISO_BACKUP"
fi
restore_spec() {
  local result=$?
  cp "$SPEC_BACKUP" sexos_build_spec.toml
  if [[ "$result" != 0 && -n "$ISO_BACKUP" ]]; then cp "$ISO_BACKUP" sexos-v1.0.0.iso; fi
  rm -f "$SPEC_BACKUP"
  if [[ -n "$ISO_BACKUP" ]]; then rm -f "$ISO_BACKUP"; fi
}
trap restore_spec EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
set_profile() {
  python3 - "$1" "$SPEC_BACKUP" <<'PY'
from pathlib import Path
import re, sys
profile = sys.argv[1]
s = Path(sys.argv[2]).read_text()
for stage in ('build_sexbell', 'build_kaleidoscope'):
    start = s.index(f'id = "{stage}"')
    end = s.index('[[stage]]', start)
    block = s[start:end]
    if not re.search(r'^rustflags = ""$', block, re.M):
        raise SystemExit(f'expected normal empty declared rustflags for {stage}')
    flags = '' if profile == 'normal' else '--cfg ' + (
        'paging_quota_proof' if profile == 'quota' and stage == 'build_kaleidoscope'
        else 'paging_exit_cleanup_proof')
    s = s[:start] + block.replace('rustflags = ""', f'rustflags = "{flags}"') + s[end:]
Path('sexos_build_spec.toml').write_text(s)
PY
}
for profile in self quota; do
  set_profile "$profile"
  GATE_DIR="$PROOF_DIR/$profile" bash scripts/master_runtime_gate.sh --probe 50 --keep-log
  quota=0; if [[ "$profile" == quota ]]; then quota=1; fi
  PAGING_LOG="$PROOF_DIR/$profile/serial.log" REQUIRE_PAGING_QUOTA="$quota" bash scripts/task_exit_cleanup_runtime_gate.sh
  cp sexos-v1.0.0.iso "$PROOF_DIR/$profile/tested.iso"
  cp iso_root/sexos-kernel "$PROOF_DIR/$profile/tested.kernel"
done
set_profile normal
GATE_DIR="$PROOF_DIR/normal" bash scripts/master_runtime_gate.sh --probe 50 --keep-log
PAGING_LOG="$PROOF_DIR/normal/serial.log" bash scripts/paging_recovery_runtime_gate.sh
cp sexos-v1.0.0.iso "$PROOF_DIR/normal/tested.iso"
cp iso_root/sexos-kernel "$PROOF_DIR/normal/tested.kernel"
echo '[paging.cleanup.profiles] PASS self_finish=1 quota_exit=1 restored_normal=1'
