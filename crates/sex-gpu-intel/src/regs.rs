// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Tiger Lake display register map: offsets inside the MMIO BAR and the bit
//! fields SexOS uses. Grouped by display block. Port numbering follows the
//! PRM: DDI A=0, B=1, (C=2 absent on TGL), Type-C TC1..TC6 = 3..8.

pub const PIPES: u32 = 4;
/// DDI ports this generation wires: A, B, TC1..TC6.
pub const PORTS: [u32; 8] = [0, 1, 3, 4, 5, 6, 7, 8];
pub fn port_name(port: u32) -> &'static str {
    ["A", "B", "C", "TC1", "TC2", "TC3", "TC4", "TC5", "TC6"].get(port as usize).copied().unwrap_or("?")
}
pub fn is_type_c(port: u32) -> bool { (3..=8).contains(&port) }

// ---- Transcoders (one per pipe on Gen12; eDP has no dedicated transcoder) ----
pub const fn trans(t: u32, reg: u32) -> u32 { 0x60000 + t * 0x1000 + reg }
pub const HTOTAL: u32 = 0x00;
pub const HBLANK: u32 = 0x04;
pub const HSYNC: u32 = 0x08;
pub const VTOTAL: u32 = 0x0c;
pub const VBLANK: u32 = 0x10;
pub const VSYNC: u32 = 0x14;
/// PIPESRC: pipe source size, (w-1)<<16 | (h-1).
pub const PIPESRC: u32 = 0x1c;
/// DP data/link M/N values (readout only).
pub const DATA_M1: u32 = 0x30;
pub const LINK_M1: u32 = 0x40;
pub const LINK_N1: u32 = 0x44;
pub const DDI_FUNC_CTL: u32 = 0x400;
pub const PSR_CTL: u32 = 0x800;
pub const PSR_STATUS: u32 = 0x840;
pub const PSR2_CTL: u32 = 0x900;

pub const FUNC_ENABLE: u32 = 1 << 31;
pub const FUNC_PORT_SHIFT: u32 = 27; // TGL: port + 1 in bits 30:27
pub const FUNC_PORT_MASK: u32 = 0xf << 27;
pub const FUNC_MODE_SHIFT: u32 = 24;
pub const FUNC_MODE_HDMI: u32 = 0;
pub const FUNC_MODE_DVI: u32 = 1;
pub const FUNC_MODE_DP_SST: u32 = 2;
pub const FUNC_MODE_DP_MST: u32 = 3;
pub const FUNC_BPC_SHIFT: u32 = 20;
pub const FUNC_PHSYNC: u32 = 1 << 16;
pub const FUNC_PVSYNC: u32 = 1 << 17;
pub fn bpc(func: u32) -> u8 { match (func >> FUNC_BPC_SHIFT) & 7 { 0 => 8, 1 => 10, 2 => 6, 3 => 12, _ => 0 } }

pub const PSR_ENABLE: u32 = 1 << 31;
pub const PSR2_ENABLE: u32 = 1 << 31;

// ---- Pipes ----
pub const fn pipe(p: u32, reg: u32) -> u32 { 0x70000 + p * 0x1000 + reg }
/// Current scanline.
pub const PIPEDSL: u32 = 0x00;
/// TRANSCONF / PIPECONF.
pub const CONF: u32 = 0x08;
pub const FRMCOUNT: u32 = 0x40;
pub const FLIPCOUNT: u32 = 0x44;
pub const CONF_ENABLE: u32 = 1 << 31;
pub const CONF_STATE: u32 = 1 << 30;
pub const DSL_MASK: u32 = 0xf_ffff;

// ---- Universal planes (1..=7 per pipe; plane 1 is primary) and cursor ----
pub const fn plane(p: u32, n: u32, reg: u32) -> u32 { 0x70180 + p * 0x1000 + (n - 1) * 0x100 + reg }
pub const PLANE_CTL: u32 = 0x00;
pub const PLANE_STRIDE: u32 = 0x08;
pub const PLANE_POS: u32 = 0x0c;
pub const PLANE_SIZE: u32 = 0x10;
/// Writing SURF arms the update: the plane latches all its registers at the next vblank.
pub const PLANE_SURF: u32 = 0x1c;
pub const PLANE_OFFSET: u32 = 0x24;
pub const PLANE_SURFLIVE: u32 = 0x2c;
pub const PLANE_COLOR_CTL: u32 = 0x4c;
pub const PLANE_ENABLE: u32 = 1 << 31;
/// ICL+ format field, bits 27:23.
pub const PLANE_FORMAT_MASK: u32 = 0x1f << 23;
pub const PLANE_FORMAT_XRGB8888: u32 = 0x4 << 24;
pub const PLANE_FORMAT_NV12: u32 = 0x0 << 24;
pub const PLANE_TILED_MASK: u32 = 0x7 << 10;
pub const PLANE_TILED_LINEAR: u32 = 0;
pub const PLANE_TILED_X: u32 = 1 << 10;
pub const PLANE_TILED_Y: u32 = 4 << 10;
/// Linear stride unit.
pub const STRIDE_UNIT: u32 = 64;
pub const SURF_ALIGN: u32 = 4096;

pub const fn cursor(p: u32, reg: u32) -> u32 { 0x70080 + p * 0x1000 + reg }
pub const CUR_CTL: u32 = 0x00;
pub const CUR_BASE: u32 = 0x04;
pub const CUR_POS: u32 = 0x08;
pub const CUR_SURFLIVE: u32 = 0x2c;
pub const CUR_MODE_MASK: u32 = 0x27;
pub const CUR_MODE_OFF: u32 = 0;
pub const CUR_MODE_64_ARGB: u32 = 0x27;
pub const CUR_MODE_128_ARGB: u32 = 0x22;
pub const CUR_MODE_256_ARGB: u32 = 0x23;
pub const CUR_POS_SIGN: u32 = 0x8000;

// ---- DDI buffers and DP AUX ----
pub const fn ddi_buf_ctl(port: u32) -> u32 { 0x64000 + port * 0x100 }
pub const DDI_BUF_ENABLE: u32 = 1 << 31;
pub const DDI_BUF_IDLE: u32 = 1 << 7;
pub const fn aux_ctl(port: u32) -> u32 { 0x64010 + port * 0x100 }
pub const fn aux_data(port: u32, i: u32) -> u32 { 0x64014 + port * 0x100 + i * 4 }
pub const AUX_DATA_REGS: u32 = 5;
pub const AUX_SEND_BUSY: u32 = 1 << 31;
pub const AUX_DONE: u32 = 1 << 30;
pub const AUX_INTERRUPT: u32 = 1 << 29;
pub const AUX_TIME_OUT_ERROR: u32 = 1 << 28;
pub const AUX_TIME_OUT_MAX: u32 = 3 << 26;
pub const AUX_RECEIVE_ERROR: u32 = 1 << 25;
pub const AUX_SIZE_SHIFT: u32 = 20;
pub const AUX_SIZE_MASK: u32 = 0x1f << 20;
pub const AUX_TBT_IO: u32 = 1 << 11;
pub const fn aux_fw_sync(n: u32) -> u32 { (n - 1) << 5 }
pub const fn aux_sync(n: u32) -> u32 { n - 1 }

// ---- GMBUS (DDC over I2C, in the PCH display block) ----
pub const GMBUS0: u32 = 0xc5100;
pub const GMBUS1: u32 = 0xc5104;
pub const GMBUS2: u32 = 0xc5108;
pub const GMBUS3: u32 = 0xc510c;
pub const GMBUS4: u32 = 0xc5110;
pub const GMBUS5: u32 = 0xc5120;
pub const GMBUS_RATE_100K: u32 = 0 << 8;
pub const GMBUS_SW_CLR_INT: u32 = 1 << 31;
pub const GMBUS_SW_RDY: u32 = 1 << 30;
pub const GMBUS_CYCLE_STOP: u32 = 4 << 25;
pub const GMBUS_CYCLE_INDEX: u32 = 2 << 25;
pub const GMBUS_CYCLE_WAIT: u32 = 1 << 25;
pub const GMBUS_BYTE_COUNT_SHIFT: u32 = 16;
pub const GMBUS_INDEX_SHIFT: u32 = 8;
pub const GMBUS_READ: u32 = 1;
pub const GMBUS_INUSE: u32 = 1 << 15;
pub const GMBUS_HW_WAIT_PHASE: u32 = 1 << 14;
pub const GMBUS_HW_RDY: u32 = 1 << 11;
pub const GMBUS_SATOER: u32 = 1 << 10;
pub const GMBUS_ACTIVE: u32 = 1 << 9;
/// GMBUS pin pair for a DDI port (ICP/TGP PCH numbering).
pub fn gmbus_pin(port: u32) -> Option<u32> {
    match port { 0 => Some(1), 1 => Some(2), 3..=8 => Some(9 + port - 3), _ => None }
}

// ---- Hotplug live status ----
/// South display engine ISR: combo DDI and Type-C live status.
pub const SDEISR: u32 = 0xc4000;
pub fn sde_hotplug_bit(port: u32) -> u32 { if is_type_c(port) { 1 << (24 + port - 3) } else { 1 << (16 + port) } }
/// North DE HPD ISR: Type-C (DP alt mode) and Thunderbolt live status.
pub const DE_HPD_ISR: u32 = 0x44470;
pub fn tc_hotplug_bit(port: u32) -> u32 { 1 << (16 + port - 3) }
pub fn tbt_hotplug_bit(port: u32) -> u32 { 1 << (port - 3) }

// ---- Panel power and backlight (PCH) ----
pub const PP_STATUS: u32 = 0xc7200;
pub const PP_CONTROL: u32 = 0xc7204;
pub const PP_ON: u32 = 1 << 31;
pub const BLC_PWM_CTL: u32 = 0xc8250;
pub const BLC_PWM_FREQ: u32 = 0xc8254;
pub const BLC_PWM_DUTY: u32 = 0xc8258;
pub const BLC_PWM_ENABLE: u32 = 1 << 31;
pub const BLC_PWM_POLARITY: u32 = 1 << 29;

// ---- Global GTT ----
/// The GGTT page table lives in the second half of the 16 MiB GTTMMADR BAR.
pub const GGTT_OFFSET: u64 = 8 << 20;
pub const GGTT_PRESENT: u64 = 1;
/// One 64-bit GGTT entry for a 4 KiB page at a physical address.
pub fn ggtt_pte(phys: u64) -> u64 { (phys & 0x7f_ffff_f000) | GGTT_PRESENT }
