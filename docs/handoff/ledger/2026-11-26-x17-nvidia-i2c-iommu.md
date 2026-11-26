# 2026-11-26 — Alienware x17 R1: SerialIO I2C0 / NVIDIA `wake_d0`, `sexgpu-nv` BAR0 & WPR2 Unload, IOMMU Sizing, Boot Heap Headroom

## Root Causes (Live x17 R1 Hardware Audit)
1. **SerialIO I2C0 (`00:15.0` `8086:43e8`) & RTX 3070 (`01:00.0` `10de:24dd`) D3hot**: `kernel/src/devmgr.rs` (`class_lease`) called `boot_memory::class_bar` *before* `device_lease::wake_d0`, and omitted `Kind::NvidiaGpu` from `wake_d0`. UEFI left `00:15.0` in D3hot, causing `invalid_bar` (`input=0`, `i2c_hid=0`) and leaving the dGPU asleep (`chip=unread`).
2. **RTX 3070 BAR0 decode & warm-boot WPR2**: `servers/sexgpu-nv/src/main.rs` read `PMC_BOOT_0` from BAR0 before enabling PCI Memory Space / Bus Master via `sex_pdx::device_command`, risking `0xFFFFFFFF` (`reads_all_ones`) when BIOS leaves Memory Decode off. On warm boots where `PFB_WPR2_ADDR_HI != 0`, `crates/sex-gpu-nv/src/boot.rs` immediately returned `Error::Wpr2Busy` instead of running `booter_unload` on SEC2.
3. **IOMMU table-frame exhaustion & low boot heap free watermark**: `kernel/src/iommu.rs` sized table frames with `(64 + services * 96).next_power_of_two()`, which under-estimated `sexdisplay` (114 frames) + `sexgpu-nv` (102 frames) (`table_frames=334/512` even without `sexgpu-nv`). On 32 GiB RAM, `kheap` grew in 4 MiB steps and left only 3.45 MiB free at `[boot.heap.budget]`.

## Changes
- `kernel/src/device_lease.rs`, `kernel/src/devmgr.rs`: Wake `SerialIo`, `Wifi`, and `NvidiaGpu` to D0 (with 10 ms PCI PM recovery stall) *before* probing `class_bar`; log raw `bar_lo`/`bar_hi`/`cmd`/`pmcsr` if `class_bar` still fails.
- `servers/sexgpu-nv/src/main.rs`: Enable `NIC_BITS` (Memory Space + Bus Master) *before* reading `PMC_BOOT_0`, and load pinned `booter_unload-535.113.01.bin` from `nvidia-gsp.pack`.
- `crates/sex-gpu-nv/src/boot.rs`, `crates/sex-gpu-nv/tests/`: Add optional `booter_unload` to `Firmware`, size `mem.booter` for both load and unload, run `booter_unload` on SEC2 when `PFB_WPR2_ADDR_HI != 0`, and add warm-boot host unit tests.
- `kernel/src/iommu.rs`: Compute IOMMU table-frame budget from actual DMA pools, 128 MiB lend windows, RMRRs, firmware domains, and context tables.
- `kernel/src/memory/kheap.rs`, `kernel/src/init.rs`: On `>= 4 GiB` RAM only (leaving 512 MiB / 1 GiB QEMU unchanged), scale growth step to 16 MiB and ensure 32 MiB free boot-heap headroom before sealing BOOT; log `limit=` in `[boot.heap.budget]`.
