// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Where GSP-RM lives in video memory, and the `GspFwWprMeta` that says so.
//!
//! From the top of VRAM down (`tu102_gsp_oneinit`):
//! VGA workspace | FRTS (1 MiB) | bootloader | GSP-RM ELF | GSP heap | meta |
//! then a 1 MiB non-WPR heap below WPR2. Booter checks all of it.

use crate::{align_down, align_up, wr64, Error};

pub const WPR_META_SIZE: usize = 256;
pub const WPR_META_MAGIC: u64 = 0xdc3a_ae21_371a_60b3;
pub const WPR_META_REVISION: u64 = 1;
pub const FRTS_SIZE: u64 = 0x10_0000;
pub const NON_WPR_HEAP_SIZE: u64 = 0x10_0000;

/// GSP-RM 535 heap sizing for LibOS 3 (GA10x, AD10x).
pub const HEAP_OS_CARVEOUT: u64 = 20 << 20;
pub const HEAP_BASE_RM: u64 = 8 << 20;
pub const HEAP_PER_GB_FB: u64 = 96 << 10;
pub const HEAP_CLIENT_ALLOC: u64 = (48 << 10) * 2048;
pub const HEAP_MIN: u64 = 84 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Range { pub addr: u64, pub size: u64 }
impl Range { pub fn end(&self) -> u64 { self.addr + self.size } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FbLayout {
    pub fb_size: u64,
    pub vga_workspace: Range,
    pub frts: Range,
    pub boot: Range,
    pub elf: Range,
    pub heap: Range,
    pub wpr2: Range,
    pub non_wpr_heap: Range,
}

/// The VGA workspace address from `PDISP_VGA_WORKSPACE_BASE` (None when the
/// display engine is absent).
pub fn vga_workspace_addr(fb_size: u64, reg: Option<u32>) -> u64 {
    let base = fb_size - 0x10_0000;
    let Some(v) = reg else { return base };
    if v & 0x8 == 0 { return base; }
    let addr = ((v & 0xffff_ff00) as u64) << 8;
    if addr < base { fb_size - 0x2_0000 } else { addr }
}

pub fn wpr_heap_size(fb_size: u64) -> u64 {
    let gb = fb_size.div_ceil(1 << 30);
    let size = HEAP_OS_CARVEOUT + HEAP_BASE_RM + align_up(HEAP_PER_GB_FB * gb, 1 << 20) + align_up(HEAP_CLIENT_ALLOC, 1 << 20);
    size.max(HEAP_MIN)
}

impl FbLayout {
    /// Lay out WPR2 for `boot_size` bytes of bootloader and `elf_size` bytes
    /// of GSP-RM image.
    pub fn compute(fb_size: u64, vga_reg: Option<u32>, boot_size: u64, elf_size: u64) -> Result<FbLayout, Error> {
        if fb_size < (512 << 20) { return Err(Error::Gsp("layout: video memory too small")); }
        let vga = vga_workspace_addr(fb_size, vga_reg);
        let mut l = FbLayout { fb_size, vga_workspace: Range { addr: vga, size: fb_size - vga }, ..Default::default() };
        let bios_addr = vga;
        l.frts = Range { addr: align_down(bios_addr, 0x2_0000) - FRTS_SIZE, size: FRTS_SIZE };
        l.boot = Range { addr: align_down(l.frts.addr - boot_size, 0x1000), size: boot_size };
        l.elf = Range { addr: align_down(l.boot.addr - elf_size, 0x1_0000), size: elf_size };
        let heap = wpr_heap_size(fb_size);
        let heap_addr = align_down(l.elf.addr - heap, 0x10_0000);
        l.heap = Range { addr: heap_addr, size: align_down(l.elf.addr - heap_addr, 0x10_0000) };
        let wpr2 = align_down(l.heap.addr - WPR_META_SIZE as u64, 0x10_0000);
        l.wpr2 = Range { addr: wpr2, size: l.frts.end() - wpr2 };
        l.non_wpr_heap = Range { addr: wpr2 - NON_WPR_HEAP_SIZE, size: NON_WPR_HEAP_SIZE };
        Ok(l)
    }

    /// `gspFwWprEnd`: the 128 KiB-aligned bottom of the VBIOS area.
    pub fn wpr_end(&self) -> u64 { align_down(self.vga_workspace.addr, 0x2_0000) }
}

/// System-memory addresses Booter needs, filled in by the boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SysmemRefs {
    pub radix3_elf: u64,
    pub bootloader: u64,
    pub bootloader_size: u64,
    pub bootloader_code_offset: u64,
    pub bootloader_data_offset: u64,
    pub bootloader_manifest_offset: u64,
    pub signature: u64,
    pub signature_size: u64,
}

/// Write `GspFwWprMeta` (offsets pinned by tests against the C layout).
pub fn write_wpr_meta(out: &mut [u8], l: &FbLayout, s: &SysmemRefs) {
    out[..WPR_META_SIZE].fill(0);
    wr64(out, 0, WPR_META_MAGIC);
    wr64(out, 8, WPR_META_REVISION);
    wr64(out, 16, s.radix3_elf);
    wr64(out, 24, l.elf.size);
    wr64(out, 32, s.bootloader);
    wr64(out, 40, s.bootloader_size);
    wr64(out, 48, s.bootloader_code_offset);
    wr64(out, 56, s.bootloader_data_offset);
    wr64(out, 64, s.bootloader_manifest_offset);
    wr64(out, 72, s.signature);
    wr64(out, 80, s.signature_size);
    wr64(out, 88, l.non_wpr_heap.addr); // gspFwRsvdStart
    wr64(out, 96, l.non_wpr_heap.addr); // nonWprHeapOffset
    wr64(out, 104, l.non_wpr_heap.size);
    wr64(out, 112, l.wpr2.addr);
    wr64(out, 120, l.heap.addr);
    wr64(out, 128, l.heap.size);
    wr64(out, 136, l.elf.addr);
    wr64(out, 144, l.boot.addr);
    wr64(out, 152, l.frts.addr);
    wr64(out, 160, l.frts.size);
    wr64(out, 168, l.wpr_end());
    wr64(out, 176, l.fb_size);
    wr64(out, 184, l.vga_workspace.addr);
    wr64(out, 192, l.vga_workspace.size);
    // bootCount, partition RPC fields, verified: all zero.
}
