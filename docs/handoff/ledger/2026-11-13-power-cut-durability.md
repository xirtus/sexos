# Power-cut durability: sexfs and FAT/exFAT checked at every cut point; FAT write order fixed

Workstream E, slice E5 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- Host, sexfs (`crates/sexfs/src/powercut.rs`, `cargo test -p sexfs --features repair`): a 120-step
  workload (writes, truncates, renames, removes, each committed) is cut after every possible number of
  device writes for 8 seeds (2,817 cuts). Unflushed writes survive in any subset, some torn mid-block.
  Every cut mounts, verifies, checks clean (`check --deep`), and holds exactly the last commit that
  returned or the one in flight.
- Host, FAT16/FAT32/exFAT (`tests/volumes_powercut.rs`, in `volumes_host_gate.sh`): the same kind of
  cuts on synced operations; every file synced before the cut-off operation reads back exactly, and
  `fsck.vfat -n` finds nothing beyond what any interrupted FAT write leaves (lost clusters, free-count
  hint, second FAT copy behind, half a long name, the in-flight file's chain longer than its size).
- Bug found and fixed in `sex-volumes` FAT: a cut could leave a directory entry pointing at clusters
  the FAT did not yet hold ("Contains a free cluster"), which a later write would have cross-linked.
  Now a growing write flushes FAT and data before the entry; remove and shrink write the entry and
  flush before freeing clusters; mkdir flushes the new folder's cluster before naming it.
- QEMU (`scripts/powercut_gate.py`, CI devices job, `POWERCUT_CYCLES` default 3, 100 for the full run):
  QEMU is killed while Spindle writes; after each kill `sexfs-fsck --deep` is clean and the next boot
  reads every committed file back.
