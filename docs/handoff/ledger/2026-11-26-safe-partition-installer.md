# 2026-11-26 — Safe Single-Partition Replacement, Fast Terminal Installer & Candy Lavender Graphical Installer

## Summary
Upgraded SexOS's installer stack (`sex-disk`, `sex-pdx`, `sex-app`, `sexfiles`, `spindle`, and `settings`) to support safe single-partition replacement on multi-boot GPT disks, an Omarchy-style fast terminal installer in Spindle, and a Candy Lavender graphical installer & partition manager in Tatami Settings.

## Changes
1. **Safe Single-Partition Replacement (`crates/sex-disk/src/gpt.rs`, `crates/sex-disk/src/install.rs`)**:
   - Added standard GPT partition type GUIDs (`TYPE_MSR`, `TYPE_BASIC_DATA`, `TYPE_WINDOWS_RE`, `TYPE_LINUX_FS`, `TYPE_LINUX_ROOT_X64`, `TYPE_LINUX_HOME`, `TYPE_LINUX_SWAP`) and `Gpt::remove(index)` to zero only a single GPT entry while leaving all other entries byte-for-byte intact.
   - Added `Mode::ReplacePartition(usize)`, `PartKind`, `PartInfo`, `ReplacedPart`, `UntouchedPart`, and `install::list_partitions`.
   - Enforced safety invariants: validates GPT entry index, refuses `TYPE_ESP` and `TYPE_MSR` (`Error::Invalid`), refuses partitions smaller than `MIN_REPLACE_SECTORS` (`1.5 GiB`, `Error::NoSpace`), carves SexOS Slot A (256 MiB) + Slot B (256 MiB) + Store strictly inside `[target.first_lba, target.last_lba]` (1 MiB aligned), preserves all other GPT entries and partition bytes untouched, and writes the updated GPT table last.
   - Updated `install::describe` to explicitly report the replaced partition `#N` and list every untouched partition (`UNTOUCHED: #...`).
   - Added comprehensive pure-Rust 4-OS multi-boot GPT disk unit test (`replace_partition_refuses_esp_msr_tiny_and_replaces_only_garuda`) in `crates/sex-disk/src/install/tests.rs`.

2. **Installer IPC & Service (`crates/sex-pdx/src/fs.rs`, `crates/sex-app/src/lib.rs`, `servers/sexfiles/src/installer.rs`, `servers/sexfiles/src/fs_service.rs`)**:
   - Extended `w::INSTALL` flags with `INSTALL_LIST`, `INSTALL_REPLACE`, and `INSTALL_LIST_RAW` (re-pinned `spec/abi_inputs.sha256` via `scripts/update_abi_hashes.sh`).
   - Updated `servers/sexfiles/src/installer.rs` to serve human-readable and machine-readable GPT partition listings and `Mode::ReplacePartition` dry-run/execute requests.
   - Allowed the system's `tatami` Settings program (`is_settings`) to invoke `w::INSTALL` in `servers/sexfiles/src/fs_service.rs` and added `sex_app::files::install`.

3. **Omarchy-Style Fast Terminal Installer (`apps/spindle/src/fs_commands.rs`, `apps/spindle/src/main.rs`)**:
   - Added `install parts` / `install list` to display every GPT partition (`#`, type, size, replaceable/LOCKED status, name) with zero heap allocations.
   - Added `install replace <N>` (dry-run plan) and `install replace <N> go` (atomic install, automatically refused over tether by existing `words.any(|w| w == b"go")` guard).

4. **Candy Lavender Graphical Installer & Partition Manager (`apps/settings/src/candy_gui.rs`, `apps/settings/src/main.rs`)**:
   - Added `"Install SexOS"` (`SidebarView::Install`) to the Candy sidebar (reachable from live boot as well).
   - Added zero-heap `InstallState`, `DiskPartKind`, and `DiskPartRow` with proportional color-coded horizontal GPT Partition Bar, clickable partition rows with visible `LOCKED` protection for ESP/MSR/`< 1.5 GiB`, green/amber safety summary panel, and `"Plan Install"` / `"Install Now"` / `"Refresh"` Candy buttons wired to `sex_app::files::install`.
   - Added host unit test `install_gui_locks_esp_msr_tiny_and_renders_partition_bar_and_buttons`.

## Verification
- `cargo test --locked -p sex-disk`: 18/18 passed (including 4-OS multi-boot GPT replacement test).
- `bash scripts/settings_host_gate.sh`: PASS.
- Host unit tests for `apps/settings/src/candy_gui.rs` + `model.rs`: 12/12 passed.
- `cargo check -Z build-std=core,alloc -Z json-target-spec --target x86_64-sex.json -p sex-disk -p spindle -p sexfiles`: PASS.
- `cargo check -Z build-std=core,alloc -Z json-target-spec --target x86_64-sex-app.json -p settings`: PASS.
