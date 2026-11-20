// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! `sexgpu-nv`, the NVIDIA GPU service (stage N4b of
//! docs/handoff/NVIDIA_GSP_BRINGUP_2026_11_19.md): what BOOT hands it.
//!
//! BOOT starts it only when the boot entry lists its module ("SexOS with NVIDIA GPU").
//! It then leaves the NVIDIA GPU powered (no D3 park) and leases it here instead of
//! to Display: a view of all of BAR0, the memory and bus-master command bits, one DMA
//! pool for GSP-RM's boot, and the firmware pack (a boot module) read only.

/// Fixed domain id, after Cufflinks (52).
pub const GPU_PD: u32 = 53;
/// Core organ: the display stack this service will feed (sexdisplay holds key 1).
pub const GPU_KEY: u8 = 1;
/// Boot module names.
pub const SERVICE_MODULE: &str = "/sexgpu-nv";
pub const FIRMWARE_MODULE: &str = "nvidia-gsp.pack";
/// The GPU's lease (also `sex_hwdb::manifest::GPU`'s slot).
pub const SLOT: u64 = 16;
/// The DMA pool: the service's ordinary pool slot.
pub const SLOT_DMA: u64 = crate::SLOT_DMA_REGION;
pub const SLOT_FIRMWARE: u64 = 24;
/// All of BAR0 on Turing to Ada.
pub const REGISTER_BYTES: u64 = 16 << 20;
pub const DMA_REGION_ID: u32 = 0x4500;
/// GSP-RM 535's boot takes about 39 MiB of GPU-visible memory (the image alone is
/// 36 MiB); the rest is room for the message queues and the first RM objects.
pub const DMA_BYTES: u64 = 64 << 20;
pub const FIRMWARE_REGION_ID: u32 = 0x4501;
/// SYS_IDENTITY query: the GPU's BAR0, BAR1 and BAR3 bus addresses and its bus/devfn,
/// which GSP-RM must be told (`GSP_SET_SYSTEM_INFO`). Answers only an NVIDIA GPU lease.
pub const QUERY_BARS: u64 = 3;

/// What [`bars`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bars { pub bar0: u64, pub bar1: u64, pub bar3: u64, pub bus: u8, pub devfn: u8 }

/// The held GPU's BAR addresses and location.
pub fn bars(slot: u64) -> Result<Bars, u64> {
    let (status, a, b, c, d): (u64, u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") crate::pci_command::SYS_IDENTITY => status, in("rdi") slot,
        inlateout("rsi") QUERY_BARS => a, inlateout("rdx") 0u64 => b,
        inlateout("r10") 0u64 => c, inlateout("r8") 0u64 => d,
        out("rcx") _, out("r11") _, options(nostack)); }
    if status != 0 { return Err(status); }
    if a == 0 || d > 0xffff { return Err(crate::pci_command::IO); }
    Ok(Bars { bar0: a, bar1: b, bar3: c, bus: (d >> 8) as u8, devfn: d as u8 })
}
