# 2026-11-26: Live USB Program Fallback & Partition Superblock FS/Label/UUID Probe

## Problem
Booting the SexOS Ventoy live USB on the Alienware x17 (with a foreign multi-OS 1 TB Micron NVMe) hit two practical gaps:
1. `install parts` (and Settings → Install SexOS) listed unnamed GPT Linux partitions without filesystem type, volume label, or UUID, making it impossible to distinguish Void Linux, Garuda Linux, old installs, or empty partitions before running `install replace <N>`.
2. On a foreign-GPT live USB boot (`State::Absent` in `sexfiles`), `/bin/` has no DiskFS files and the NVMe has no SexOS program store, so `run flash`, `run +net kaleidoscope`, `run +net tailor`, `run tatami`, and Spotlight/SilkBar app launches failed with `No program by that name.` (and Spotlight/SilkBar did not grant `+net` or launch real `kaleidoscope`/`loom`/`tatami`).

## Changes
1. **Read-Only Superblock Probe (`crates/sex-disk/src/install.rs`, `servers/sexfiles/src/installer.rs`)**:
   - Extended `PartInfo` with `fs_type: Option<&'static str>`, `fs_label: String`, `fs_uuid: Option<[u8; 16]>`, and `PartInfo::short_uuid()` (first 4 bytes as 8 lowercase hex chars).
   - Added `probe_fs()` in `list_partitions()` to safely read-only inspect ext2/3/4 (offset `1024`), btrfs (offset `65536`), Linux swap (`SWAPSPACE2` at `4086`), NTFS, exFAT, FAT32/FAT16/FAT12, and all-zero `"empty"` partitions.
   - Updated `servers/sexfiles/src/installer.rs` to display `FS`, volume label, and short UUID in `install parts`, and include concise FS/label/UUID info in `INSTALL_LIST_RAW` for Settings → Install SexOS.
   - Added unit test `superblock_probe_detects_filesystems_labels_and_uuids` in `crates/sex-disk/src/install/tests.rs`.
2. **Live USB `/bin/<name>` Program Fallback (`servers/sexfiles/src/volumes.rs`, `fs_service.rs`, `program_store.rs`, `apps/spindle/src/fs_commands.rs`)**:
   - Added read-only `volumes::has_program()` and `volumes::read_program()` searching mounted `/media/<vol>/programs/<name>` (e.g., `/media/Ventoy/programs/kaleidoscope`).
   - Added `fs_service::read_media_program()` and allowed read-only `PREPARE` for `/bin/<name>` when `State::Absent` and a mounted `/media/<vol>/programs/<name>` exists.
   - Updated `program_store::try_load()` to fall back to `fs_service::read_media_program()` when DiskFS `/bin/<name>` and the disk program store are absent or missing the program.
   - Preserved `RUN_NETWORK` / run flags in Spindle's bare-name `OP_RUN` fallback (`apps/spindle/src/fs_commands.rs`).
3. **Spotlight & SilkBar Network Grants (`servers/silk-shell/src/spotlight_glue.rs`, `silkbar_menus_glue.rs`)**:
   - Granted `+net` (`PROGRAM_NETWORK = true`) when launching network apps (`kaleidoscope`, `tailor`, `flirt`, `gossip`, `sexshop`) from Spotlight or SilkBar.
   - Updated Browser/Files/Settings launch dispatch to try real `kaleidoscope` (+net), `loom`, and `tatami` with built-in fallbacks, without touching `servers/silk-shell/src/main.rs`.

## Verification
- `cargo test --locked -p sex-disk`: 19 passed, 0 failed.
- `bash scripts/settings_host_gate.sh`: `[settings.host.gate] PASS`.
- `PYTHONPATH=scripts/tests python3 -m unittest scripts/tests/test_passive_wait_observer.py scripts/tests/test_self_wait_source.py`: 25 passed.
- `rustc --edition 2021 --test tests/spotlight.rs`: 13 passed.
- `cargo check -Z build-std=core,alloc -Z json-target-spec --target x86_64-sex.json -p sex-disk -p sexfiles -p silk-shell -p spindle`: passed.
- `cargo check -Z build-std=core,alloc -Z json-target-spec --target x86_64-sex-app.json -p settings`: passed.
