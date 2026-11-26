# 2026-11-26: Alienware x17 R1 hardware bring-up fixes (audio, Bluetooth, ACPI battery/lid, log ring)

Live x17 R1 boot log (`sexwork/tether/logs/current.log`) showed four concrete hardware/observability issues:

1. **Intel Tiger Lake-H Audio (`00:1f.3` `8086:43c8`, class `040100`) — `[bell.sound.absent] reason=no_lease`**
   - Root cause: `crates/sex-hwdb/src/manifest.rs` (`BELL`) only matched PCI class `04:03` (`HD Audio`). Tiger Lake-H cAVS (`8086:43c8`) reports class `04:01` (multimedia audio/DSP) while exposing standard HDA MMIO registers on BAR0.
   - Fix: added `Match::vendor_class(0x8086, 0x04_01_00, CLASS, Lease::Hda, 20)` to `BELL`, matched Intel `04:01` in `sex-hwdb` `by_class`, included `Kind::Hda` in `devmgr::class_lease`'s `wake_d0` pass, and updated `crates/sex-hwdb/tests/x17r1.rs` to assert `manifest::claim` on `8086:43c8` (`04:01:00`).

2. **Intel AX210 Bluetooth (`8087:0032` USB port 14) — `[cufflinks.firmware] missing=ibt-0041-0041.sfi pack=1` -> `[cufflinks.idle] controller=none`**
   - Root cause: `sexusb` enumerated `8087:0032` and `cufflinks` decoded `variant=0x17 bootloader=1 file=ibt-0041-0041`, but `scripts/sexos_build_trace.sh` never ran `scripts/fetch_bt_firmware.sh`, and `scripts/make_wifi_firmware_pack.py` only globbed `iwlwifi-*` (skipping `ibt-*` even though its `build()` already kept `ibt-*` stored uncompressed for `sex_bt::pack::find`).
   - Fix: updated `scripts/make_wifi_firmware_pack.py` to accept `--bt-firmware` (default `target/bt-firmware`) and include `ibt-*` stored uncompressed alongside `iwlwifi-*`; updated `scripts/sexos_build_trace.sh`'s `wifi_firmware` stage to run `scripts/fetch_bt_firmware.sh target/bt-firmware`; added `scripts/tests/test_make_wifi_firmware_pack.py`.

3. **ACPI Battery & Lid Readback (`[sexacpi.sensors]` `battery=-1 ... lid=-1` with `batteries=1 ac=1 lids=1`)**
   - Root cause: vendored `acpi-6.1.1` (`vendor/acpi-6.1.1/src/aml/mod.rs`) returned `Err(AmlError::LibUnimplemented)` on:
     - `do_store` to `ReferenceKind::RefOf` — which `Index(PBST, i)` / `Index(PBIF, i)` / `Index(buf, i)` produce, so standard laptop `_BST`/`_BIF`/`_BIX` methods (`Store(val, Index(PBST, 0))`) always failed silently.
     - `Opcode::CopyObject`, `FieldUnitKind::Index` (`IndexField`), and `FieldUnitKind::Bank` (`BankField`).
   - Fix: implemented `ReferenceKind::RefOf` and direct object targets in `do_store`, `Opcode::CopyObject`, and `FieldUnitKind::Bank`/`FieldUnitKind::Index` in `vendor/acpi-6.1.1/src/aml/mod.rs` (documented in `vendor/acpi-6.1.1/SEXOS_CHANGES.md` with host unit tests); updated `servers/sexacpi/src/{main,laptop}.rs` to unwrap transparent references, fall back from `_BIX` to `_BIF` if `_BIX` yields no usable reading, and log one-shot `[sexacpi.battery.diag]` / `[sexacpi.lid.diag]` on failure.

4. **High-Frequency Kernel Log Ring Spam (`[silkbar.civil.*]` / `[clock.source.snapshot]`)**
   - Root cause: `servers/silkbar/src/main.rs` sampled the CMOS civil clock every ~0.5 s on bare metal (`62 * 8 = 496` ticks at 1000 Hz), emitting ~5–10 lines/sec and wrapping the 576 KiB kernel log ring in ~15 minutes even though the top bar displays `HH:MM`.
   - Fix: kept immediate sampling for the first 2 boot samples (so all QEMU civil clock gates see `sequence=1, 2` immediately with exact consecutive ordinals/sequences), throttled steady-state sampling to every 20 cadences (~10 s, ~20x quieter), and raised `HEALTH_TICKS` in `crates/silkbar-model/src/civil_clock.rs` from `620` to `30_000`.
