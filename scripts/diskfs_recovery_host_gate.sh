#!/usr/bin/env bash
# Host regression of production DiskFS recovery/allocation functions.
# Descriptor I/O, cache invalidation and manifest persistence are test doubles.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-diskfs-recovery.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
vfs = (root / 'servers/sexfiles/src/vfs.rs').read_text()
pdx = (root / 'crates/sex-pdx/src/lib.rs').read_text()
messages = (root / 'servers/sexfiles/src/messages.rs').read_text()
def unique(pattern, source):
    matches = re.findall(pattern, source, re.M | re.S)
    if len(matches) != 1:
        raise SystemExit(f'production extraction failed: {pattern}: {len(matches)} matches')
    return matches[0]
parts = ['mod sex_pdx {']
for name in ['DISKFS_V4_POOL_BASE_LBA', 'DISKFS_V4_BLOCK_SECTORS', 'DISKFS_V4_POOL_BLOCKS','SPINDLE_HISTORY_BASE_LBA','SPINDLE_HISTORY_SECTORS']:
    parts.append(unique(r'^pub const ' + name + r':[^\n]+;', pdx))
parts.append(unique(r'^pub const fn ranges_overlap\b.*?^}', pdx))
parts.append('pub mod document {pub const IO:u32=9;pub const INVALID:u32=1;pub const STALE:u32=3;}')
parts.append('}')
parts.append('mod messages {')
for name in ['ERR_FULL', 'ERR_BAD_CMD', 'ERR_EXISTS', 'ERR_PERM_DENIED', 'ERR_NOT_FOUND', 'ERR_OVERFLOW']:
    parts.append(unique(r'^pub const ' + name + r':[^\n]+;', messages))
parts.append('}')
for name in ['DISKFS_SLOTS', 'DISKFS_V3_NAME_MAX', 'DISKFS_V3_SYSTEM_SLOTS', 'DISKFS_V4_BLOCK_SECTORS',
             'DISKFS_V4_POOL_BASE_LBA', 'DISKFS_V4_POOL_BLOCKS', 'DISKFS_V4_MAX_EXTENTS', 'V4_EMPTY']:
    parts.append(unique(r'^(?:pub )?const ' + name + r':[^\n]+;', vfs))
parts.append('#[derive(Clone, Copy, Default)]\n' + unique(r'^struct V4Extent \{[^}]+\}', vfs))
parts.append('#[derive(Clone, Copy)]\n' + unique(r'^struct V4Entry \{[^}]+\}', vfs))
parts.append(unique(r'^static DOCUMENT_WRITE_LOCK:[^\n]+;',vfs))
parts.append(unique(r'^fn document_locked\b.*?^}',vfs))
for name in ['v4_in_use', 'v4_bitmap_test', 'v4_bitmap_set', 'v4_bitmap_clear',
             'v4_free_pool_only', 'v4_allocate', 'v4_bitmap_rebuild',
             'v4_unpack_name', 'handle_diskfs_create', 'handle_diskfs_delete']:
    parts.append(unique(r'^fn ' + name + r'\b.*?^\}', vfs))
parts.append(unique(r'^struct DocumentGrowth \{.*?^}',vfs))
parts.append(unique(r'^impl DocumentGrowth \{.*?^}',vfs))
diskfs=(root/'servers/sexfiles/src/backends/diskfs.rs').read_text()
parts.append('const SEXFS_V0_DATA_LBA_MIN:u64=128; const SEXFS_V0_DATA_LBA_MAX:u64=2019; const DISKFS_BLOCK_SIZE:usize=4096;')
parts.append(unique(r'^struct SexfsObjectEntryV0Parsed \{[^}]+\}', diskfs))
for name in ['sexfs_v0_recompute_freemap_checksum','sexfs_v0_freemap_alloc_block_in_range','sexfs_v0_freemap_alloc_specific','sexfs_v0_validate_extent_bounds']:
    parts.append(unique(r'^fn '+name+r'\b.*?^\}',diskfs))
for name in ['sexfs_v0_write_data_sector','sexfs_v0_write_full_block']:
    body=unique(r'^fn '+name+r'\b.*?^\}',diskfs)
    assert body.index('sex_pdx::ranges_overlap')<body.index('diskfs_bridge_get_buf_va')
pathlib.Path(sys.argv[2]).write_text('\n\n'.join(parts))
PY
DISKFS_RECOVERY_SOURCE="$TEST_DIR/production.rs" rustc --edition=2021 --test \
  "$ROOT_DIR/tests/diskfs_recovery.rs" -o "$TEST_DIR/recovery-tests"
"$TEST_DIR/recovery-tests"
echo '[gate.diskfs_recovery_host.result] PASS scope=production_functions_mocked_io'
