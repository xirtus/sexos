# SexOS filesystem check and repair; Files repairs a damaged disk at start

Workstream E, slice E6 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- `crates/sexfs/src/check.rs` (feature `repair`): `check(dev, deep)` reads without writing and reports;
  `repair(dev, mem, deep)` salvages every readable leaf record, settles them against each other and
  writes a new tree beside the old one, then a superblock: a power cut during repair leaves the last
  commit as it was. A clean filesystem is never written. Faults handled: unreadable/misplaced nodes
  (records the newest commit lost are taken from the commit before, data re-verified), entries to
  missing inodes, second names for a folder or file, duplicate names (`~N`), wrong entry kinds,
  unnamed inodes (to `/lost+found` as `#ID`, folders with their contents), block records past the end
  of a file / for a missing inode / outside the disk / shared with another file, data blocks failing
  their checksum (reported; dropped from files taken back from the older commit).
- `sexfiles`: a disk that fails to mount as corrupt is repaired at start (`[files.fs.repaired] ...`)
  instead of leaving the system without its files.
- `tools/sexfs-fsck` (host tool): `sexfs-fsck [--repair] [--deep] [--offset SECTORS] IMAGE`, exit 0 clean,
  1 damaged/repaired, 2 unreadable. Used by the gate; also recovers a SexOS disk from Linux.
- The plan named a program `apps/fsck`; the filesystem belongs to the Files server, so repair runs
  there (at start) and as a host tool. An on-demand "check now" in Files is a follow-up.
- Tests: `cargo test -p sexfs --features repair` (8 host tests with injected faults: dangling entry,
  orphaned folder tree, stray and shared blocks, damaged leaf newest-only and shared by both commits in
  a 300-file tree, bad data, both superblocks gone). QEMU gate `scripts/sexfs_repair_gate.py`: newest
  root node damaged on the host, fsck says damaged, Files repairs at boot, the file reads back, new
  writes work, `sexfs-fsck --deep` clean afterwards.
