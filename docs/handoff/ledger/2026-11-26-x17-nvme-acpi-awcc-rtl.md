# 2026-11-26: Alienware x17 R1 NVMe, ACPI, AWCC & RTL8125 Fixes

## Root Causes & Changes

1. **NVMe (`apps/sexdrive/src/main.rs`)**:
   - **IRQ busy-loop (`sleeps=8691` for 64 wakes, `turns=475` in 0 ticks)**: `sys_wait_self_ticks(1)` returns immediately without parking whenever `pd.message_ring` has a queued IPC message (e.g. `sexfiles` calling during boot self-tests or back-to-back requests), which `nvme_wait_completion`/`nvme_wait_batch` cannot consume while DMA is in flight. Both wait loops now check `irq_take()` after `sys_wait_self_ticks(1)`, only count genuine 1-tick sleeps or IRQ wakes in `NVME_IRQ_WAITS`, switch to `spin_loop` + `sys_yield()` when `sys_wait_self_ticks(1)` returns in the same tick with no IRQ, and drain stale IRQ bits on completion.
   - **2 MiB loaned transfers (`mdts=9`)**: Added `LOAN_CMD_MAX = 512 * PAGE_SIZE` (2 MiB, fitting 511 PRP entries in one 4 KiB PRP list page) and `NVME_LOAN_CMD_RUN = identify::max_transfer(drive.mdts, LOAN_CMD_MAX)`. Decoupled `prp2_for_loan` and zero-copy `nvme_read_into_mapped_va` from `BLOCK_CMD_RUN` (128 KiB) while keeping bounced transfers capped at `BLOCK_CMD_RUN`.
   - **Range-check & logging bugs**: Replaced hardcoded `WRITE_PROOF_LBA` with `disk_sectors().saturating_sub(1)` for `max_lba` in read/write command logging, logged `(1u32 | cq_ien) as u64` instead of `1u64` in `create_cq.submit`, and logged `io_entries` instead of `16` in `ioq.ready`.

2. **ACPI (`servers/sexacpi/src/main.rs`)**:
   - Logged `[sexacpi.table.err] index={} oem={} len={} err={:?}` when `aml.load_table` fails instead of silently discarding `Err(e)`.
   - Added `_CST` diagnostics (`CstDiag`) and `[sexacpi.idle.fallback]` logging whenever `FW_IDLE` / `_CST` falls back to `idle=hlt`.

3. **AWCC WMI (`crates/sex-laptop/src/awcc.rs`, `servers/sexacpi/src/wmi.rs`)**:
   - Changed `Reading.rpm` from `[u16; MAX_RESOURCES]` to `[Option<u16>; MAX_RESOURCES]` so `OP_FAN_RPM` firmware refusal (`None` -> logged as `-1`) is no longer masked as `0 RPM` (`Some(0)`).
   - Raised `plausible_celsius` floor from `1..=125` to `10..=125` to filter bogus `2°C` D3cold dGPU readings.

4. **Realtek RTL8125 (`servers/sexnet/src/ethernet.rs`, `tests/sexnet_realtek.rs`)**:
   - Added `Driver::link_up()` and checked link status in `Driver::send()` before submitting TX descriptors. When link is down (`Some(false)`), TX drops cleanly with `SendError::NotReady` without arming `self.pending` or tripping `Failure::Deadline` -> `Phase::Quarantined`, so plugging in a cable later brings the NIC right back up.

5. **PCI Hardware DB (`crates/sex-hwdb/src/lib.rs`, `crates/sex-hwdb/tests/x17r1.rs`)**:
   - Added exact entry `(0x8086, 0x43a4)` as `"Tiger Lake-H SPI Flash"` (`Service::Kernel`, `Support::Platform`) so `hw` census no longer misclassifies it as a planned `sexinput` controller via class `0c8000`.
