#!/usr/bin/env bash
# Other systems' filesystems (FAT, exFAT, ext4, NTFS) on the host: fixture images made by
# each system's own mkfs, and, where the host has the tools, its own fsck after our writes.
# Usage: volumes_host_gate.sh [TEST_NAME_FILTER]
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-volumes.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --crate-type=rlib --crate-name sex_volumes -O -C overflow-checks=on -C debug-assertions=on \
  "$ROOT_DIR/crates/sex-volumes/src/lib.rs" --out-dir "$TEST_DIR"
for t in "$ROOT_DIR"/tests/volumes_*.rs; do
  name="$(basename "$t" .rs)"
  [ "$name" = volumes_common ] && continue
  rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code -L "$TEST_DIR" \
    --extern sex_volumes="$TEST_DIR/libsex_volumes.rlib" "$t" -o "$TEST_DIR/$name"
  "$TEST_DIR/$name" ${1:-}
done
echo "[volumes.host.gate] PASS"
