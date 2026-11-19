// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! BAR0 register offsets used by the GA10x GSP boot. Names follow NVIDIA's
//! open headers (`dev_*.h` in open-gpu-kernel-modules) where they exist.

/// Chip identity: architecture, implementation, revision.
pub const PMC_BOOT_0: u32 = 0x0000_0000;
/// Newer chip identity word (Ampere onwards fills it too).
pub const PMC_BOOT_42: u32 = 0x0000_0a00;
/// Per-engine enable bits (GA100+ "device enable").
pub const PMC_DEVICE_ENABLE: u32 = 0x0000_0600;

/// Software scratch registers the boot ucodes report errors in.
pub const fn pbus_sw_scratch(i: u32) -> u32 { 0x0000_1400 + i * 4 }
/// FWSEC-FRTS writes its error code in the top half of scratch 0xe.
pub const FRTS_ERR_SCRATCH: u32 = pbus_sw_scratch(0xe);
/// FWSEC-SB (secure boot) writes its error code in the low half of 0x15.
pub const SB_ERR_SCRATCH: u32 = pbus_sw_scratch(0x15);

/// Mirror of the GPU's own PCI configuration space inside BAR0.
pub const PCI_CONFIG_MIRROR: u32 = 0x0008_8000;
pub const PCI_CONFIG_MIRROR_SIZE: u32 = 0x1000;
/// Bit 0 of mirror offset 0x50: 1 = BAR0's ROM window shows the shadow copy,
/// 0 = it reads the ROM chip itself.
pub const PCI_ROM_SHADOW: u32 = PCI_CONFIG_MIRROR + 0x50;

/// The ROM window (1 MiB): the VBIOS straight from the SPI flash.
pub const PROM: u32 = 0x0030_0000;
pub const PROM_SIZE: u32 = 0x0010_0000;

/// Usable video memory in MiB (GA102 and later).
pub const PFB_VIDMEM_SIZE_MB: u32 = 0x0011_83a4;
/// Display engine: where firmware put the VGA workspace (bit 3 = valid).
pub const PDISP_VGA_WORKSPACE_BASE: u32 = 0x0062_5f04;

/// WPR2 bounds (4 KiB units in bits 31:4); HI != 0 means WPR2 exists.
pub const PFB_WPR2_ADDR_LO: u32 = 0x001f_a824;
pub const PFB_WPR2_ADDR_HI: u32 = 0x001f_a828;

/// Fuse versions per falcon ucode id, used to choose which signature to patch.
pub const FUSE_SEC2_UCODE_VERSION: u32 = 0x0082_4140;
pub const FUSE_NVDEC_UCODE_VERSION: u32 = 0x0082_4100;
pub const FUSE_GSP_UCODE_VERSION: u32 = 0x0082_41c0;

/// SEC2 resume handshake bit the CPU sequencer's CORE_RESUME waits on.
pub const SEC2_RESUME_STATUS: u32 = 0x0011_80f8;

/// Falcon engine bases on GA10x.
pub const GSP_BASE: u32 = 0x0011_0000;
pub const SEC2_BASE: u32 = 0x0084_0000;

/// Registers relative to a falcon's base.
pub mod falcon {
    pub const IRQSCLR: u32 = 0x004;
    pub const IRQSTAT: u32 = 0x008;
    pub const IRQMCLR: u32 = 0x014;
    pub const MAILBOX0: u32 = 0x040;
    pub const MAILBOX1: u32 = 0x044;
    pub const ITFEN: u32 = 0x048;
    /// "OS" register: the GSP-RM app version goes here before it runs.
    pub const OS: u32 = 0x080;
    /// "RM" register: the driver writes `PMC_BOOT_0` here after a reset.
    pub const RM: u32 = 0x084;
    pub const HWCFG2: u32 = 0x0f4;
    pub const CPUCTL: u32 = 0x100;
    pub const BOOTVEC: u32 = 0x104;
    pub const DMACTL: u32 = 0x10c;
    pub const DMATRFBASE: u32 = 0x110;
    pub const DMATRFMOFFS: u32 = 0x114;
    pub const DMATRFCMD: u32 = 0x118;
    pub const DMATRFFBOFFS: u32 = 0x11c;
    pub const DMATRFBASE1: u32 = 0x128;
    pub const CPUCTL_ALIAS: u32 = 0x130;
    pub const ENGINE_RESET: u32 = 0x3c0;
    pub const FBIF_TRANSCFG0: u32 = 0x600;
    pub const FBIF_CTL: u32 = 0x624;
    /// Command queue head 0: GSP-RM's doorbell for new RPCs.
    pub const QUEUE_HEAD0: u32 = 0xc00;

    /// Second register page (falcon2 / RISC-V), `base + ADDR2 + x`.
    pub const ADDR2: u32 = 0x1000;
    pub const BROM_MOD_SEL: u32 = 0x180;
    pub const BROM_CURR_UCODE_ID: u32 = 0x198;
    pub const BROM_ENGIDMASK: u32 = 0x19c;
    pub const BROM_PARAADDR0: u32 = 0x210;
    pub const RISCV_CPUCTL: u32 = 0x388;
    pub const RISCV_BCR_CTRL: u32 = 0x668;

    pub const CPUCTL_STARTCPU: u32 = 1 << 1;
    pub const CPUCTL_HALTED: u32 = 1 << 4;
    pub const CPUCTL_ALIAS_EN: u32 = 1 << 6;
    pub const HWCFG2_RESET_READY: u32 = 1 << 31;
    pub const HWCFG2_MEM_SCRUBBING: u32 = 1 << 12;
    pub const RISCV_CPUCTL_ACTIVE: u32 = 1 << 7;
    pub const DMATRFCMD_IDLE: u32 = 1 << 1;
}
