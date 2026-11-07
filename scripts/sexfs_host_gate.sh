#!/usr/bin/env bash
# The filesystem against a model, power cuts and damaged blocks, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sexfs.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code "$ROOT_DIR/tests/sexfs.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
# Negative controls: each broken ordering must be caught by the power-cut test.
mutant() {
  mkdir -p "$TEST_DIR/$1/tests" "$TEST_DIR/$1/crates/sexfs/src"
  cp "$ROOT_DIR/tests/sexfs.rs" "$TEST_DIR/$1/tests/"
  python3 - "$ROOT_DIR/crates/sexfs/src/lib.rs" "$TEST_DIR/$1/crates/sexfs/src/lib.rs" "$2" "$3" <<'PY'
import sys
s=open(sys.argv[1]).read();assert s.count(sys.argv[3])>=1,sys.argv[3]
open(sys.argv[2],'w').write(s.replace(sys.argv[3],sys.argv[4],1))
PY
  rustc --edition=2021 --test -O -A dead_code -A warnings "$TEST_DIR/$1/tests/sexfs.rs" -o "$TEST_DIR/$1/t" 2>/dev/null
  if "$TEST_DIR/$1/t" --test-threads=1 a_power_cut >/dev/null 2>&1; then echo "[sexfs.negative] FAIL survived=$1"; exit 1; fi
  echo "[sexfs.negative] rejected=$1"
}
mutant no_flush_before_superblock 'for i in 0..self.mem.slots.len() {
            if self.mem.slots[i].dirty { self.write_slot(i)?; }
        }
        self.dev.flush().map_err(|_| Error::Io)?;' 'for i in 0..self.mem.slots.len() {
            if self.mem.slots[i].dirty { self.write_slot(i)?; }
        }'
mutant overwrite_in_place 'if gen_of(&self.mem.cache[i]) == self.tx { self.mem.slots[i].dirty = true; return Ok((block, i)); }' 'self.mem.slots[i].dirty = true; return Ok((block, i));'
mutant one_superblock 'self.dev.write(generation % SB_SLOTS, self.mem.scratch)' 'self.dev.write(0, self.mem.scratch)'
echo "[sexfs.host.gate] PASS scope=model_crash_damage_host_not_real_disk"
