# Driver fuzzing (device input)

A driver must never crash or poll forever on what a device tells it: register
values, DMA rings and descriptors, USB/HID descriptors and reports, codec answers,
firmware files and ACPI tables. Laptops ship buggy firmware, USB devices can be
malicious, and a flaky or unplugged device reads garbage or all ones. Every target
here builds the **real driver source** (its crate, or its file by `#[path]`) against a
**device model** fed by the fuzz input.

| Target | Driver (source) | What the input controls |
|---|---|---|
| `drv_wifi_rx` | Intel Wi-Fi, `crates/sex-wifi` | RX buffers from firmware: notifications, command answers, received MPDUs; 802.11 beacons, auth, assoc, deauth, RSN, EAPOL, CCMP, WPA3 SAE commit/confirm from an access point |
| `drv_wifi_firmware` | `crates/sex-wifi` | `.ucode` TLV files, `.pnvm` files, the firmware pack |
| `drv_uvc` | webcams, `crates/sex-uvc` (sexuvc) | UVC class descriptors, probe/commit answer, isochronous payload stream, YUY2/NV12/MJPEG frames |
| `drv_display_intel` | Intel display, `crates/sex-gpu-intel` + `sex-kms` | every register read (AUX/DPCD, GMBUS, pipes, planes, hotplug, backlight, vblank), monitor EDID |
| `drv_gpu_nv` | NVIDIA, `crates/sex-gpu-nv` | VBIOS through the PROM window, image chain, BIT/PMU tables, FWSEC; firmware headers and ELF; the GSP message queue in shared memory |
| `drv_iommu_tables` | `crates/sex-iommu` | DMAR and IVRS tables, VT-d capability registers, fault and event records |
| `drv_hid_touch` | I2C-HID touchpads, `crates/sex-hid-touch` | every I2C read: HID descriptor, report descriptor, input reports through the gesture recognizer |
| `drv_i2c_dw` | DesignWare I2C, `crates/sex-i2c-dw` | status, FIFO levels, abort sources, received bytes |
| `drv_imsm_raid` | Intel RAID metadata, `crates/sex-imsm` (sexdrive) | the metadata at a disk's end, drive serials, which members are present |
| `drv_laptop_ec` | EC, battery, `_CRS`, `crates/sex-laptop` (sexacpi) | EC status/data ports, resource buffers, `_BST`/`_BIF`/`_BIX` packages, thermal readings |
| `drv_hda` | HDA sound, `servers/sexbell/src/hda.rs` | a controller + codec model: every codec answer (widget graph, connections, pin configs, jack sense), unsolicited and dropped responses, stream positions, garbage register reads |
| `drv_xhci_usb` | USB, `servers/sexusb/src/{xhci,usb,hid}.rs` | an xHCI model: ports and speeds, command completions and slot ids, every byte devices return (descriptors, HID reports, hub status), short packets, stalls, stray events, unplug |
| `drv_nvme` | NVMe, `apps/sexdrive/src/{controller_ready,completion_wait,apst}.rs` | CSTS values, completion queue entries, a misbehaving clock, the APST table in Identify |
| `drv_acpi_aml` | ACPI, the `acpi` crate as sexacpi runs it | FADT, AML (DSDT) and every operation-region read (report-only, see below) |

## Running

```
./run.sh                          # every target, 20 s each, built-in mutator (any toolchain)
DRIVER_FUZZ_SEED=7 ./run.sh drv_hda
cargo run --release --bin drv_hda -- crashes/drv_hda-*.bin    # replay (prints the panic)
DRIVER_FUZZ_LOG=1 cargo run --release --bin drv_xhci_usb -- FILE   # with the driver's log lines
./run_libfuzzer.sh 300 drv_xhci_usb   # coverage guided (nightly toolchain + cargo-fuzz)
```

CI (`.github/workflows/driver-fuzz.yml`): pull requests touching a driver run the
smoke pass; nightly runs every target under libFuzzer for 20 minutes with the corpus
kept between nights. A failing input is uploaded as an artifact.

Overflow checks are on for our code (the image builds without them, so an overflow
there silently wraps a length or index); third-party crates run as in the image.

## How the models work

* **Register crates** (`sex-gpu-intel`, `sex-gpu-nv`, `sex-i2c-dw`, `sex-wifi`, `sex-laptop`)
  already take their hardware through a trait; the target implements it with
  `FuzzRegs`: each read is the next input word. Past the input the device either fell
  off the bus (all ones) or hung (zeros), picked by the input's first byte.
* **Server drivers** that touch MMIO directly get it through a hook: `sexusb` calls
  crate-level `mmio_read32`/`trb_write_volatile`, which the target defines; `hda.rs`
  switches its six register accessors under `cfg(device_model)` (set by `build.rs`).
  `pdx-model/` stands in for `sex_pdx`: the real crate's types, with the kernel calls
  replaced (a model clock that moves on every look, absent services, quiet logs).
* **Hangs:** every device read calls `tick()`; more than 4 million reads for one input
  is reported as `DRIVER_HANG`. A driver's own poll budgets must end before that.

Add a target: a `fuzz_targets/drv_NAME.rs` with `run(&[u8])`, `seeds()` and
`driver_target!(run, seeds)`, a `[[bin]]` in `Cargo.toml`, a row above.

## Findings so far (fixed in the same change)

| Driver | Input | Was | Fix |
|---|---|---|---|
| Intel display readout | timing register with reserved bits set | `u16` count wrapped to 0 (later divides) | 13-bit fields masked |
| I2C DesignWare | RX-not-empty before any read was queued | `received` ran past reads issued; subtraction wrapped, transfer spun to its 2M-poll budget | only take bytes for queued reads |
| I2C DesignWare | controller gone (reads all ones) | every transfer spun 2M polls (~seconds) | all-ones status ends the transfer at once |
| Battery (`sex-laptop`) | `_BST`/`_BIF` integers wider than 32 bits | sums and percent overflowed | wider values count as unknown |
| Thermal | huge `_TMP` | `i32` overflow | computed in `i64` |
| NVIDIA VBIOS | PMU table pointer near `u32::MAX` | offset wrapped to the image start | checked adds |
| NVIDIA firmware ELF | section header offsets near `u32::MAX` | `usize` add overflowed (panic) | checked adds |
| HDA codec graph | dense mixer graph; endless unsolicited responses | up to 16^5 lookups; verb wait never ended | walk budget of 512 visits per search; 256 unsolicited skips per verb |
| sexacpi | FADT with PM1 blocks in system memory | `map_physical_region` panic: sexacpi stopped at boot | refused with `[sexacpi.fadt.refused]` |

## Known findings (open)

* **`acpi` 6.1.1 AML interpreter** (`drv_acpi_aml`, report-only; the target builds
  sexacpi's own copy, `vendor/acpi-6.1.1`): malformed AML reaches
  `todo!()` for `Match` and `CopyObject` (both appear in real DSDTs), a `panic!()` in
  unary maths, an out-of-range slice for a method whose length runs past its table,
  and `assert_ne!(path, root)` in the namespace. In sexacpi any of these stops the
  service. Plan: change `vendor/acpi-6.1.1` to return `AmlError` instead (listed in
  its `SEXOS_CHANGES.md`), send the fixes upstream, then make the target blocking.
* The NVMe target covers the controller, completion and APST code; the full queue
  model follows once the NVMe queue rework (PR #432 and the per-core queue work) lands.
