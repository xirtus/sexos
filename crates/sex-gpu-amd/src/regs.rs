// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DCN register offsets and fields. AMD headers give each register as a dword
//! offset inside a segment of the DCN block; every register used here is in
//! segment 2, whose base is 0x34C0 dwords on every listed chip (renoir,
//! yellow_carp and sienna_cichlid `DCN_BASE`). Byte offset in the BAR =
//! (0x34C0 + reg) * 4. Instances repeat at fixed strides.

use crate::asic::Layout;

/// DCN segment 2 base, in dwords.
pub const SEG2: u32 = 0x34C0;
pub const fn at(reg: u32) -> u32 { (SEG2 + reg) * 4 }

pub const OTG_STRIDE: u32 = 0x80;
pub const HUBP_STRIDE: u32 = 0xDC;
pub const AUX_STRIDE: u32 = 0x1C;

// OTG (output timing generator), instance 0. DCN 3.5 moved the block after V_TOTAL.
pub const OTG_H_TOTAL: u32 = 0x1B2A;
pub const OTG_H_BLANK_START_END: u32 = 0x1B2B;
pub const OTG_H_SYNC_A: u32 = 0x1B2C;
pub const OTG_H_SYNC_A_CNTL: u32 = 0x1B2D;
pub const OTG_V_TOTAL: u32 = 0x1B2F;
pub const OTG_STATUS_POSITION: u32 = 0x1B4A;

/// The OTG registers whose offset depends on the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtgLate { pub v_blank_start_end: u32, pub v_sync_a: u32, pub v_sync_a_cntl: u32, pub control: u32, pub frame_count: u32 }
pub const fn otg_late(l: Layout) -> OtgLate {
    match l {
        Layout::Dcn35 => OtgLate { v_blank_start_end: 0x1B38, v_sync_a: 0x1B39, v_sync_a_cntl: 0x1B3A, control: 0x1B43, frame_count: 0x1B4D },
        _ => OtgLate { v_blank_start_end: 0x1B36, v_sync_a: 0x1B37, v_sync_a_cntl: 0x1B38, control: 0x1B41, frame_count: 0x1B4C },
    }
}
pub const fn otg(i: u32, reg: u32) -> u32 { at(reg + i * OTG_STRIDE) }

pub const OTG_MASTER_EN: u32 = 1 << 0;
pub const OTG_CURRENT_MASTER_EN_STATE: u32 = 1 << 16;
pub const FIELD15: u32 = 0x7FFF;
/// H/V sync polarity bit: 1 = active low.
pub const SYNC_POL_NEGATIVE: u32 = 1;
pub const FRAME_COUNT_MASK: u32 = 0x00FF_FFFF;

// HUBP (pipe fetching the surface) and HUBPREQ, instance 0.
pub const HUBP_SURFACE_CONFIG: u32 = 0x05E5;
pub const HUBP_TILING_CONFIG: u32 = 0x05E7;
pub const HUBP_PRI_VIEWPORT_START: u32 = 0x05E9;
pub const HUBP_PRI_VIEWPORT_DIMENSION: u32 = 0x05EA;
pub const HUBP_CNTL: u32 = 0x05F3;
pub const HUBPREQ_SURFACE_PITCH: u32 = 0x0607;
pub const HUBPREQ_PRIMARY_SURFACE_ADDRESS: u32 = 0x060A;
pub const HUBPREQ_PRIMARY_SURFACE_ADDRESS_HIGH: u32 = 0x060B;
pub const HUBPREQ_FLIP_CONTROL: u32 = 0x061B;
pub const fn hubp(i: u32, reg: u32) -> u32 { at(reg + i * HUBP_STRIDE) }

pub const PIXEL_FORMAT_MASK: u32 = 0x7F;
/// SURFACE_PIXEL_FORMAT values (DC's hubp1_program_pixel_format).
pub const PIXEL_FORMAT_RGB565: u32 = 3;
pub const PIXEL_FORMAT_ARGB8888: u32 = 8;
pub const PIXEL_FORMAT_ARGB2101010: u32 = 10;
pub const SW_MODE_MASK: u32 = 0x1F;
pub const SW_MODE_LINEAR: u32 = 0;
pub const HUBP_BLANK_EN: u32 = 1 << 0;
pub const HUBP_VTG_SEL_SHIFT: u32 = 4;
pub const HUBP_VTG_SEL_MASK: u32 = 0xF << HUBP_VTG_SEL_SHIFT;
pub const FIELD14: u32 = 0x3FFF;
pub const ADDRESS_HIGH_MASK: u32 = 0xFFFF;
pub const SURFACE_UPDATE_LOCK: u32 = 1 << 0;
/// 0 = flip at vsync, 1 = immediate.
pub const SURFACE_FLIP_TYPE: u32 = 1 << 1;
pub const SURFACE_FLIP_PENDING: u32 = 1 << 8;

// DCN VM aperture: where VRAM (or the APU's carve-out) sits in the display's address space.
pub const fn fb_location_base(l: Layout) -> u32 { at(match l { Layout::Dcn20 => 0x0493, _ => 0x0475 }) }
pub const fn fb_location_top(l: Layout) -> u32 { fb_location_base(l) + 4 }
pub const fn fb_offset(l: Layout) -> u32 { fb_location_base(l) + 8 }
/// FB_BASE, FB_TOP and FB_OFFSET count 16 MiB units.
pub const FB_FIELD: u32 = 0x00FF_FFFF;
pub const FB_UNIT_SHIFT: u32 = 24;

// DP AUX engine, instance 0.
pub const AUX_CONTROL: u32 = 0x1F50;
pub const AUX_SW_CONTROL: u32 = 0x1F51;
pub const AUX_ARB_CONTROL: u32 = 0x1F52;
pub const AUX_INTERRUPT_CONTROL: u32 = 0x1F53;
pub const AUX_SW_STATUS: u32 = 0x1F54;
pub const AUX_SW_DATA: u32 = 0x1F56;
pub const fn aux(i: u32, reg: u32) -> u32 { at(reg + i * AUX_STRIDE) }

pub const AUX_EN: u32 = 1 << 0;
pub const AUX_RESET: u32 = 1 << 4;
pub const AUX_RESET_DONE: u32 = 1 << 5;
pub const AUX_SW_GO: u32 = 1 << 0;
pub const AUX_SW_START_DELAY_MASK: u32 = 0xF << 4;
pub const AUX_SW_WR_BYTES_SHIFT: u32 = 16;
pub const AUX_SW_WR_BYTES_MASK: u32 = 0x1F << AUX_SW_WR_BYTES_SHIFT;
pub const AUX_REG_RW_CNTL_STATUS_SHIFT: u32 = 2;
pub const AUX_REG_RW_CNTL_STATUS_MASK: u32 = 3 << AUX_REG_RW_CNTL_STATUS_SHIFT;
/// AUX_REG_RW_CNTL_STATUS values.
pub const ARB_SW: u32 = 1;
pub const ARB_DMCU: u32 = 2;
pub const AUX_SW_USE_AUX_REG_REQ: u32 = 1 << 16;
pub const AUX_SW_DONE_USING_AUX_REG: u32 = 1 << 17;
pub const AUX_SW_DONE_ACK: u32 = 1 << 1;
pub const AUX_SW_DONE: u32 = 1 << 0;
pub const AUX_SW_RX_TIMEOUT_STATE: u32 = 0x70;
pub const AUX_SW_RX_TIMEOUT: u32 = 0x80;
pub const AUX_SW_HPD_DISCON: u32 = 1 << 9;
pub const AUX_SW_RX_INVALID: u32 = (1 << 14) | (1 << 20) | (1 << 22) | (1 << 23);
pub const AUX_SW_REPLY_BYTE_COUNT_SHIFT: u32 = 24;
pub const AUX_SW_REPLY_BYTE_COUNT_MASK: u32 = 0x1F << AUX_SW_REPLY_BYTE_COUNT_SHIFT;
pub const AUX_SW_DATA_RW: u32 = 1 << 0;
pub const AUX_SW_DATA_SHIFT: u32 = 8;
pub const AUX_SW_DATA_MASK: u32 = 0xFF << AUX_SW_DATA_SHIFT;
pub const AUX_SW_INDEX_SHIFT: u32 = 16;
pub const AUX_SW_AUTOINCREMENT_DISABLE: u32 = 1 << 31;
