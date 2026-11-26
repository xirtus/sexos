# 2026-11-26: Alienware x17 R1 Live Fixes — ACPI Lid Reference Unwrapping, Zero-Heap Spindle `sweep`, and `00:15.0` Unassigned BAR0 Analysis

Branch: `gemini/x17-live-fixes-wave4` (based on `3faecab1d`).

## 1. ACPI `_LID` `ObjectNotOfExpectedType { expected: Integer, got: Integer }` Fix

- **Root cause**: In `vendor/acpi-6.1.1/src/aml/object.rs`, `Object::typ()` already unwraps `Object::Reference { inner, .. }` recursively, whereas `Object::as_integer()`, `as_string()`, `as_buffer()`, `to_integer()`, and `to_buffer()` matched only direct variants. Likewise, in `vendor/acpi-6.1.1/src/aml/mod.rs`, `Opcode::Increment`/`Decrement` (`gain_mut`) and `do_field_write` matched `Object::Integer`/`Buffer` directly without unwrapping references (such as `Local0` / `ArgN` or `RefOf` slots). When AML evaluated `_LID` or incremented/decremented a local variable, it failed with `ObjectNotOfExpectedType { expected: Integer, got: Integer }`.
- **Fix**:
  - `vendor/acpi-6.1.1/src/aml/object.rs`: Recursively unwrap `Object::Reference { inner, .. }` in `as_integer`, `as_string`, `as_buffer`, `to_integer`, and `to_buffer`.
  - `vendor/acpi-6.1.1/src/aml/mod.rs`: Unwrap references before `gain_mut` in `Opcode::Increment | Opcode::Decrement` and before matching value variants in `do_field_write`.
  - Documented in `vendor/acpi-6.1.1/SEXOS_CHANGES.md`.
  - Added host unit test `reference_unwrapping_and_local_inc_dec` in `crates/sex-laptop/tests/awcc_aml.rs`.

## 2. Live Microkernel Server Hot-Swap Status (`servers/sexacpi`)

- Built `servers/sexacpi` with exact SexOS release flags (`RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie" cargo build --locked --manifest-path servers/sexacpi/Cargo.toml --target x86_64-sex.json -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem --release`).
- Confirmed live x17 tether connection in `/Users/xmacbookm5/Projects/Sex/sexwork/tether/logs/state.json` (`"connected": true`, peer `192.168.1.39`).
- **Blocker for live `push-server`**: The running `/Users/xmacbookm5/Projects/Sex/sexwork/tether/tether_host.py` is the older script version (no `@HOTSWAP` / `push-server` support; `grep -n HOTSWAP` returned no matches). Per safety rules, the running `tether_host.py serve` was left undisturbed. Copying `scripts/tether_host.py` to `sexwork/tether/tether_host.py` when restarting the tether host enables live `push-server sexacpi ...` hot-swap without rebooting the x17.

## 3. Spindle `sweep` Heap OOM Fix (`apps/spindle/src/sweep_command.rs`, `sweep_math.rs`)

- **Root cause**: `apps/spindle/src/main.rs` intentionally uses `DummyAllocator` (`alloc()` always returns `null_mut()`) to enforce Spindle's bounded zero-heap-growth contract. Consequently, every `Vec::try_reserve_exact` in `sweep_command.rs` (`measure()`, `run_ipc()`, `auto_pick_file()`, `bench_read_chunk()`, `run_nvme()`) failed immediately with `out of heap memory` on the live x17.
- **Fix**:
  - Reduced `DEFAULT_ITERS` from `500` to `64` in `apps/spindle/src/sweep_math.rs` (asserted in `tests/spindle_sweeps.rs`).
  - Eliminated **all** heap allocations (`alloc::vec::Vec`) in `apps/spindle/src/sweep_command.rs`, replacing them with bounded stack scratch (`[u64; 256]` = 2 KiB for strided percentile sampling, `[u8; w::PATH_BYTES]` = 1 KiB for file paths) and zero-copy page-touching directly over the `sexfiles` shared region for `4KiB`, `16KiB`, and `64KiB` sequential NVMe reads. Zero static BSS buffers were added.
  - Added direct `STAT` fallback auto-discovery for `/media/Ventoy/sexos-x17-c28152a0.iso`, `/media/Ventoy/wifi-sweep.txt`, and `/media/windows/Windows/win.ini` when top-level `LIST` finds no regular files.

## 4. Trackpad `00:15.0` (`8086:43e8`) Unassigned 64-bit BAR0 Analysis

- **Observed state on x17**: Intel Tiger Lake-H Serial IO I2C #0 at `00:15.0` (`8086:43e8`) is left by UEFI with 64-bit BAR0 unassigned (`bar_lo=0x00000004`, `bar_hi=0x00000000`, `cmd=0x0004` — Bus Master set, Memory Space decode off).
- **Kernel audit**:
  - `kernel/src/boot_memory.rs` (`class_bar`) calls `PciDevice::probe_boot_bar_any(0)`, which decodes the BAR address (`0`) and rejects an unassigned BAR.
  - `kernel/src/hotplug.rs` (`arrive` + `sex_hotplug::plan`) only assigns BARs *behind* PCIe hot-plug root/downstream ports where the bridge's own PCI config window registers (`0x20..0x2c`) define the safe MMIO aperture.
  - `00:15.0` sits on **root bus 0** (no upstream PCI-to-PCI bridge window registers). Guessing a physical MMIO address in ring 0 without host-bridge `_CRS` risks colliding with ECAM, stolen GPU memory, SMRAM, ACPI NVS, or PCH hidden BARs, and violates SexOS's design guardrail keeping AML out of ring 0 (`docs/DESIGN_GUARDRAILS.md` §1).
- **Recommended microkernel strategy** (zero kernel line-budget growth here):
  1. **Ring 3 (`servers/sexacpi`)**: Parse `\_SB.PCI0._CRS` (`DWordMemory` / `QWordMemory` descriptors) to obtain the host bridge's legal 32-bit and above-4G 64-bit MMIO apertures, then subtract all assigned PCI BARs, PCI bridge windows, ECAM, and Limine/E820 reserved/RAM regions to pick a 4 KiB-aligned free address (preferring the above-4G 64-bit `_CRS` window for 64-bit BARs like `00:15.0`).
  2. **Ring 0 (minimal mechanism)**: Accept a validated BAR-placement request (or BOOT-time reserved aperture) to program BAR0 (`0x10`/`0x14`) on `00:15.0` with decode off, verify readback, enable `PCI_COMMAND_MEMORY` (`0x0002`), and bind the `SerialIo` MMIO lease for the I2C HID touchpad driver.

## 5. Verification

- `cargo test -p sex-laptop`: 53 passed (44 unit + 7 `awcc_aml` + 2 `x17_trackpad_acpi_sweep`).
- `tests/spindle_sweeps.rs`: 6 passed (including `DEFAULT_ITERS == 64`).
- `python3 scripts/check_architectural_truth.py`: PASS.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts/tests/test_kernel_design_guardrails.py`: 7 passed (kernel untouched, line budget unchanged).
- `servers/sexacpi` (`x86_64-sex.json` release) and `apps/spindle` (`x86_64-sex.json` check): clean build.
