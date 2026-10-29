# DiskFS zero-length recovery reservation — 2026-10-28

## Result and scope

A production DiskFS recovery defect was reproduced before repair: after a crash
between indirect-descriptor commit and manifest-size commit, an empty object's
committed block was treated as free. A second object's allocation returned the
same LBA (400). Recovery now reserves valid descriptors even when the manifest
size is zero. A fresh empty object with no readable descriptor remains valid.
Creation also clears any previous slot descriptor before publishing a new
object identity, and failed delete persistence restores the prior live entry.

This is a host regression against **verbatim production recovery and allocation
functions**, with descriptor I/O, cache invalidation and manifest persistence
replaced by test doubles.
It is not a live NVMe crash test, a hardware persistence guarantee, an epoch
implementation, or whole-system replay evidence.

## Failure mechanism

`servers/sexfiles/src/vfs.rs::handle_diskfs_write` persists the grown indirect
extent descriptor before it publishes the new manifest size. The existing
`extent_committed` marker names that crash boundary. Rebooting at this point can
legitimately produce an in-use slot whose manifest size is zero and whose valid
indirect descriptor references a pool block.

Previously, `v4_bitmap_rebuild` skipped every size-zero object before reading its
descriptor. `v4_allocate` therefore offered the referenced block to another slot.
A subsequent write to the original slot used `v4_cache_get` and reused its
committed descriptor, aliasing the second object's content. This contradicts the
reservation claim in `DISKFS_V4_CRASH_INJECTION_V1.md`.

The bounded repair removes the early size-zero skip. Descriptor reads now occur
for every in-use slot. Only a failed descriptor read on a size-zero object is
accepted as a fresh empty object; a valid descriptor goes through the existing
extent validation and bitmap reservation logic. Nonempty objects with unreadable
descriptors retain the existing fail-closed drop behavior.

## Reproducer and verification

Command: `scripts/diskfs_recovery_host_gate.sh`

Before the runtime edit, the initial six tests produced **4 passed, 2 failed**,
exit 101. The two failures were:

- `crash_before_size_commit_reserves_extent_for_empty_owner`: owned pool block 0
  was absent from the rebuilt bitmap.
- `allocation_after_crash_cannot_alias_empty_owners_descriptor`: original and
  newly allocated extents both started at LBA 400.

After the initial repair and negative controls: **9 passed, 0 failed**, exit 0.
The other cases cover fresh empty objects, committed nonempty objects, unreadable
nonempty descriptors, deleted slots, duplicate descriptors, misaligned extents,
and idempotent recovery of an empty object's valid reservation.

Independent integration review then reproduced a lifecycle regression in the
initial repair. A lower slot can have a stale descriptor after DELETE commits
its free manifest entry but crashes before descriptor invalidation. Another
already-live higher slot can allocate the released block. CREATE previously
reused the lower slot without clearing its old descriptor; the new rebuild
reserved that stale extent first, dropped the higher live owner as overlapping,
and persisted the drop. A temporary test with empty slot 3 and nonempty slot 4
both describing block 0 failed on the initial patch and passed with the original
size-zero skip restored. Restoring the skip was not retained because it would
reintroduce the original crash-A alias.

The supplementary lifecycle repair calls `v4_indirect_write(i, buf_va, &[], 0)`
while the selected CREATE slot is still free. The write's existing read-back
verification must succeed before cache invalidation, name/generation assignment,
or manifest persistence. Failure returns the error without publishing the new
entry. This preserves valid live-empty crash-A descriptors while discarding
descriptors from a previous identity at slot reuse.

DELETE now saves the live entry and restores it if `v4_persist` fails, before
bitmap release, descriptor reset, or cache invalidation. Otherwise a failed
delete could expose a still-owned slot to the new CREATE reset within the same
boot. This is in-memory rollback on a reported I/O failure; it does not establish
whether an ambiguous failed write reached durable storage.

The gate now also extracts production CREATE, DELETE and name decoding, plus
their actual error constants. Four added tests first produced **9 passed,
4 failed**, exit 101, before the supplementary runtime edits. After repair:
**13 passed, 0 failed**, exit 0. Added coverage:

- Reused lower empty slot cannot evict a higher committed block owner on rebuild.
- Descriptor reset and cache invalidation precede new manifest publication.
- Failed descriptor reset leaves the slot's old name/generation and descriptor
  unchanged in the fixture, without a manifest call.
- Failed DELETE persistence preserves live identity and bitmap reservation,
  without descriptor reset or cache invalidation.

`bash -n scripts/diskfs_recovery_host_gate.sh` and the scoped `git diff --check`
also passed. The parent mission owns sealed image build and integration checks.

The gate extracts the exact named production functions and canonical layout
constants at invocation time; it fails if extraction is missing or ambiguous or
if the extracted Rust no longer compiles. It does not carry a rewritten copy of
the recovery algorithm. `tests/diskfs_recovery.rs` supplies only the fixture
state, descriptor-I/O/persist/cache-invalidation doubles, and host lock adapter. The test arrays
represent already-read metadata, not invented system execution state.

## Existing live evidence and next runtime regression

The existing `scripts/diskfs_v4_crash_injection_gate.sh` tests old size zero at
crash point A, then immediately regrows the same object. That ordering cannot
expose this alias: no competing object claims the accidentally free block first.
Its `A_no_layout_overlap` marker checks static region layout, not overlap between
object extents. Existing historical PASS results therefore did not cover this
failure.

A suitable extension is to land crash point A, reboot, create and fill a second
object with distinct content, then regrow the original object; verify both exact
contents before and after a further reboot. No new epoch engine is needed.
The present patch does not claim this live scenario was run.
The stale-descriptor lifecycle also needs a real delete-crash/reuse/reboot test.
Already-conflicting descriptors on an older image have no on-disk generation
binding that can identify the rightful owner; this change prevents the new
CREATE path from publishing that ambiguity rather than repairing arbitrary
previously corrupted images.

## Remaining architectural truth

- SexDisplay currently mutates registry/clock state and writes the live
  framebuffer directly. Its cursor trace explicitly treats draw completion as
  present. This is useful input-path observability, not snapshot → transform →
  swap → commit or atomic scanout evidence.
- No active UiIR or epoch engine implementation was found in the inspected Rust
  sources. `kernel/src/desktop/replay.rs::replay` only restores `workspace_id`.
- DiskFS writes existing content in place. The existing empty-old/final-new
  crash scenarios do not prove atomic old-or-new publication for arbitrary
  multi-chunk replacement saves or torn sectors. This patch repairs one
  concrete recovery ownership boundary; those broader guarantees remain open.
