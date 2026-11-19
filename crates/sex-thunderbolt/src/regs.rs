// SPDX-License-Identifier: GPL-3.0-only
//! NHI registers (BAR0) and the integrated controller's vendor config words.

/// 16 bytes per hop: descriptor base (64-bit), producer/consumer word at +8,
/// descriptor count at +12 (RX: plus the frame size in the top half).
pub const TX_RING_BASE: u32 = 0x00000;
pub const RX_RING_BASE: u32 = 0x08000;
/// 32 bytes per hop: ring flags at +0, (RX) SOF/EOF masks at +4.
pub const TX_OPTIONS_BASE: u32 = 0x19800;
pub const RX_OPTIONS_BASE: u32 = 0x29800;
/// Three bit fields (TX, RX, RX overflow), one bit per hop.
pub const RING_NOTIFY_BASE: u32 = 0x37800;
/// Two bit fields (RX, TX) enabling each ring's interrupt.
pub const RING_INTERRUPT_BASE: u32 = 0x38200;
/// Hop count in the low 11 bits, interface version in 23:16.
pub const CAPS: u32 = 0x39640;
pub const CAPS_HOPS_MASK: u32 = 0x7ff;
pub const CAPS_VERSION_2: u32 = 0x40;
pub const DMA_MISC: u32 = 0x39864;
pub const RESET: u32 = 0x39898;
pub const INMAIL_DATA: u32 = 0x39900;
pub const INMAIL_CMD: u32 = 0x39904;
pub const OUTMAIL_CMD: u32 = 0x3990c;
/// Firmware status: the ICM is enabled (bit 0) and has checked its NVM (bit 31).
pub const FW_STS: u32 = 0x39944;
pub const FW_STS_ICM_EN: u32 = 1 << 0;
pub const FW_STS_NVM_AUTH_DONE: u32 = 1 << 31;
/// Registers a driver touches end here; the BAR is 256 KiB.
pub const REGISTERS_END: u32 = 0x39948;

pub const RING_FLAG_RAW: u32 = 1 << 30;
pub const RING_FLAG_ENABLE: u32 = 1 << 31;

/// Descriptor flags (12 bits at bit 20 of the descriptor's third word).
pub const DESC_CRC_ERROR: u32 = 0x1;
pub const DESC_COMPLETED: u32 = 0x2;
pub const DESC_POSTED: u32 = 0x4;
pub const DESC_BUFFER_OVERRUN: u32 = 0x4;
pub const DESC_INTERRUPT: u32 = 0x8;

/// Control frames: 256 bytes on the wire, the last four a CRC-32C.
pub const FRAME_SIZE: usize = 256;

/// Integrated (Ice Lake and later) NHI vendor words in PCI config space. BOOT,
/// which owns config space, raises FORCE_POWER when it leases the NHI (Linux
/// `icl_nhi_force_power`), then the firmware sets FW_READY.
pub mod config {
    pub const VS_CAP_9: u16 = 0xc8;
    pub const VS_CAP_9_FW_READY: u32 = 1 << 31;
    pub const VS_CAP_22: u16 = 0xfc;
    pub const VS_CAP_22_FORCE_POWER: u32 = 1 << 1;
    pub const VS_CAP_22_DMA_DELAY_SHIFT: u32 = 24;
    /// DMA delay Linux programs with force power.
    pub const VS_CAP_22_DMA_DELAY: u32 = 0x22;
    /// The PCI class of a USB4 host interface.
    pub const CLASS_USB4_NHI: u32 = 0x0c_03_40;

    /// VS_CAP_22 with force power raised (`on`) or dropped.
    pub fn force_power(vs_cap_22: u32, on: bool) -> u32 {
        if on { (vs_cap_22 & 0x00ff_ffff) | VS_CAP_22_DMA_DELAY << VS_CAP_22_DMA_DELAY_SHIFT | VS_CAP_22_FORCE_POWER }
        else { vs_cap_22 & !VS_CAP_22_FORCE_POWER }
    }
}
