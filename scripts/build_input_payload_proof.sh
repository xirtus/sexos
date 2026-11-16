#!/usr/bin/env bash
# Explicit UNSAFE diagnostic artifact, then restore/rebuild the normal image.
# Never type passwords or private content into the diagnostic artifact.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
if [[ $# != 1 ]]; then echo 'usage: build_input_payload_proof.sh EMPTY_EVIDENCE_DIRECTORY' >&2; exit 2; fi
mkdir -p "$1"
EVIDENCE_DIR="$(cd "$1" && pwd)"
if [[ -n "$(ls -A "$EVIDENCE_DIR")" ]]; then echo 'evidence directory must be empty' >&2; exit 1; fi
INPUT_PROFILE_BACKUP="$(mktemp -d "${TMPDIR:-/tmp}/sexos-input-profile.XXXXXX")"
cp sexos_build_spec.toml "$INPUT_PROFILE_BACKUP/spec"
for artifact in sexos-v1.0.0.iso iso_root/sexos-kernel iso_root/servers/sexinput iso_root/servers/silk-shell; do
  if [[ -f "$artifact" ]]; then mkdir -p "$INPUT_PROFILE_BACKUP/$(dirname "$artifact")"; cp "$artifact" "$INPUT_PROFILE_BACKUP/$artifact"; fi
done
cleanup() {
  result=$?;trap - EXIT
  cp "$INPUT_PROFILE_BACKUP/spec" sexos_build_spec.toml
  if [[ "$result" != 0 ]]; then
    for artifact in sexos-v1.0.0.iso iso_root/sexos-kernel iso_root/servers/sexinput iso_root/servers/silk-shell; do
      if [[ -f "$INPUT_PROFILE_BACKUP/$artifact" ]]; then cp "$INPUT_PROFILE_BACKUP/$artifact" "$artifact"; else rm -f "$artifact"; fi
    done
  fi
  rm -rf "$INPUT_PROFILE_BACKUP";exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
python3 scripts/input_payload_profile.py --output "$EVIDENCE_DIR/diagnostic-spec.toml"
cp "$EVIDENCE_DIR/diagnostic-spec.toml" sexos_build_spec.toml
GATE_DIR="$EVIDENCE_DIR/diagnostic-build" bash scripts/master_runtime_gate.sh --probe 60 --keep-log > "$EVIDENCE_DIR/diagnostic-build.log" 2>&1
python3 scripts/freeze_native_evidence.py --build "$EVIDENCE_DIR/diagnostic-build" --out "$EVIDENCE_DIR/frozen" --iso sexos-v1.0.0.iso --kernel iso_root/sexos-kernel
cp sexos-v1.0.0.iso "$EVIDENCE_DIR/UNSAFE-input-payload-proof.iso"
printf '%s\n' 'UNSAFE diagnostic image: records keyboard contents; never enter passwords or private data.' > "$EVIDENCE_DIR/README.txt"
cp "$INPUT_PROFILE_BACKUP/spec" sexos_build_spec.toml
bash scripts/entrypoint_build.sh > "$EVIDENCE_DIR/normal-build.log" 2>&1
cmp sexos_build_spec.toml "$INPUT_PROFILE_BACKUP/spec"
cp sexos-v1.0.0.iso "$EVIDENCE_DIR/normal.iso"
cp sexos_build_spec.toml "$EVIDENCE_DIR/normal-spec.toml"
