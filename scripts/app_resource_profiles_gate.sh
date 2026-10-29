#!/usr/bin/env bash
# Two sealed builds: declared app-resource proof, then the exact normal spec.
# The final normal image also drives the existing three-app GUI close proof.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
PROOF_DIR="${GATE_DIR:-/tmp/sexos_app_resource_profiles}"
mkdir -p "$PROOF_DIR"
if [[ -n "$(ls -A "$PROOF_DIR")" ]]; then
  echo 'app resource profile evidence directory must be empty' >&2
  exit 1
fi
PROOF_DIR="$(cd "$PROOF_DIR" && pwd)"
BACKUP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-app-resource-profile.XXXXXX")"
cp sexos_build_spec.toml "$BACKUP_DIR/spec"
HAD_ISO=0
HAD_KERNEL=0
if [[ -f sexos-v1.0.0.iso ]]; then cp sexos-v1.0.0.iso "$BACKUP_DIR/iso"; HAD_ISO=1; fi
if [[ -f iso_root/sexos-kernel ]]; then cp iso_root/sexos-kernel "$BACKUP_DIR/kernel"; HAD_KERNEL=1; fi
restore() {
  local result=$?
  trap - EXIT
  cp "$BACKUP_DIR/spec" sexos_build_spec.toml
  if [[ "$result" != 0 ]]; then
    if [[ "$HAD_ISO" == 1 ]]; then cp "$BACKUP_DIR/iso" sexos-v1.0.0.iso; else rm -f sexos-v1.0.0.iso; fi
    if [[ "$HAD_KERNEL" == 1 ]]; then cp "$BACKUP_DIR/kernel" iso_root/sexos-kernel; else rm -f iso_root/sexos-kernel; fi
  fi
  rm -rf "$BACKUP_DIR"
  exit "$result"
}
trap restore EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# Preserve every other declaration and hash. Only Kaleido's declared stage cfg
# changes; the baseline's existing Quil persistence cfg is a normal declaration.
python3 - "$BACKUP_DIR/spec" <<'PY'
from pathlib import Path
import re, sys
baseline = Path(sys.argv[1]).read_text()
blocks = list(re.finditer(r'^\[\[stage\]\]\s*\n.*?(?=^\[\[stage\]\]|\Z)', baseline, re.M | re.S))
normal = ('build_kernel', 'build_sexinput', 'build_sexbell', 'build_kaleidoscope')
selected = None
for stage in normal:
    matches = [m for m in blocks if re.search(r'^id = "' + stage + r'"$', m.group(), re.M)]
    if (len(matches) != 1
        or len(re.findall(r'^rustflags =', matches[0].group(), re.M)) != 1
        or len(re.findall(r'^rustflags = ""$', matches[0].group(), re.M)) != 1):
        raise SystemExit(f'expected one normal empty declared rustflags for {stage}')
    if stage == 'build_kaleidoscope': selected = matches[0]
assert selected is not None
profile = selected.group().replace('rustflags = ""', 'rustflags = "--cfg app_resource_cleanup_proof"', 1)
Path('sexos_build_spec.toml').write_text(baseline[:selected.start()] + profile + baseline[selected.end():])
PY
GATE_DIR="$PROOF_DIR/resource" ISO="sexos-v1.0.0.iso" bash scripts/master_runtime_gate.sh --probe 50 --keep-log
APP_RESOURCE_LOG="$PROOF_DIR/resource/serial.log" bash scripts/app_resource_runtime_gate.sh
cp sexos-v1.0.0.iso "$PROOF_DIR/resource/tested.iso"
cp iso_root/sexos-kernel "$PROOF_DIR/resource/tested.kernel"
cp sexos_build_spec.toml "$PROOF_DIR/resource/tested-spec.toml"
cp "$BACKUP_DIR/spec" sexos_build_spec.toml
GATE_DIR="$PROOF_DIR/normal" ISO="sexos-v1.0.0.iso" bash scripts/master_runtime_gate.sh --probe 50 --keep-log
PAGING_LOG="$PROOF_DIR/normal/serial.log" bash scripts/paging_recovery_runtime_gate.sh
cp sexos-v1.0.0.iso "$PROOF_DIR/normal/tested.iso"
cp iso_root/sexos-kernel "$PROOF_DIR/normal/tested.kernel"
cp sexos_build_spec.toml "$PROOF_DIR/normal/tested-spec.toml"
GATE_DIR="$PROOF_DIR/desktop-close" ISO="$ROOT_DIR/sexos-v1.0.0.iso" bash scripts/desktop_close_runtime_gate.sh
# Close proof must use, and leave in place, the verified normal artifact.
cmp sexos_build_spec.toml "$BACKUP_DIR/spec"
cmp sexos-v1.0.0.iso "$PROOF_DIR/normal/tested.iso"
cmp iso_root/sexos-kernel "$PROOF_DIR/normal/tested.kernel"
echo '[app.resource.profiles] PASS resource_cleanup=1 restored_normal=1 desktop_close_apps=3'
