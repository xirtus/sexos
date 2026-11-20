#!/usr/bin/env bash
# The filesystem against a model, power cuts and damaged blocks, on the host.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sexfs.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test -O -C overflow-checks=on -A dead_code "$ROOT_DIR/tests/sexfs.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
# A volume laid out by the allocator sexfs had before tree blocks moved to the top of
# the disk (2026-11-19): the same on-disk format, tree blocks between data. Written by
# this test file built against that allocator, then mounted, read and grown by today's.
mkdir -p "$TEST_DIR/legacy/tests" "$TEST_DIR/legacy/crates/sexfs/src"
cp "$ROOT_DIR/tests/sexfs.rs" "$TEST_DIR/legacy/tests/"
# Every other sexfs module as it is; only lib.rs gets the old allocator below.
cp "$ROOT_DIR"/crates/sexfs/src/*.rs "$TEST_DIR/legacy/crates/sexfs/src/"
python3 - "$ROOT_DIR/crates/sexfs/src/lib.rs" "$TEST_DIR/legacy/crates/sexfs/src/lib.rs" <<'PY'
import sys
s=open(sys.argv[1]).read()
for head,old in (('fn alloc_meta(&mut self) -> Result<u64> {\n','return self.alloc();'),
                 ('fn alloc_data(&mut self, hint: u64) -> Result<u64> {\n','let _ = hint; return self.alloc();')):
    assert s.count(head)==1,head
    s=s.replace(head,head+'        '+old+'\n')
open(sys.argv[2],'w').write(s)
PY
rustc --edition=2021 --test -O -A dead_code -A warnings "$TEST_DIR/legacy/tests/sexfs.rs" -o "$TEST_DIR/legacy/t"
SEXFS_WRITE_LEGACY_IMAGE="$TEST_DIR/legacy.img" "$TEST_DIR/legacy/t" --exact legacy_layout_image_is_written_when_asked --nocapture 2>&1 | grep -E '^\[sexfs\.layout\]|test result' || true
test -s "$TEST_DIR/legacy.img"
SEXFS_LEGACY_IMAGE="$TEST_DIR/legacy.img" "$TEST_DIR/tests" --exact a_volume_laid_out_the_old_way_mounts_reads_and_grows --nocapture 2>&1 | grep -E '^\[sexfs\.legacy\]'
SEXFS_LEGACY_IMAGE="$TEST_DIR/legacy.img" "$TEST_DIR/tests" --exact a_volume_laid_out_the_old_way_mounts_reads_and_grows >/dev/null
# Negative controls: each broken ordering must be caught by the power-cut test.
mutant() {
  mkdir -p "$TEST_DIR/$1/tests" "$TEST_DIR/$1/crates/sexfs/src"
  cp "$ROOT_DIR/tests/sexfs.rs" "$TEST_DIR/$1/tests/"
  cp "$ROOT_DIR"/crates/sexfs/src/*.rs "$TEST_DIR/$1/crates/sexfs/src/"
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
