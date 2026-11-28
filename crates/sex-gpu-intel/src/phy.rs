// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Combo PHY (DDI A/B) transmit levels: the voltage swing and pre-emphasis
//! DisplayPort link training asks for, per lane.
//!
//! Register blocks from the TGL PRM Vol 2c (PORT_CL_DW*, PORT_PCS_DW1,
//! PORT_TX_DW2/4/5/7, lane and group views). The swing tables are the PRM's
//! "Combo PHY DP/eDP voltage swing programming" values for Tiger Lake-H,
//! cross-checked with the MIT-licensed i915 `tgl_combo_phy_trans_dp_hbr*`
//! and `icl_combo_phy_trans_dp_hbr2_edp_hbr3` tables (BRIDGE_LEDGER t2).

use crate::Mmio;

pub const fn base(phy: u32) -> u32 { if phy == 0 { 0x162000 } else { 0x6c000 } }
pub const fn cl_dw(phy: u32, dw: u32) -> u32 { base(phy) + 4 * dw }
pub const fn pcs_grp(phy: u32, dw: u32) -> u32 { base(phy) + 0x600 + 4 * dw }
pub const fn pcs_ln(phy: u32, ln: u32, dw: u32) -> u32 { base(phy) + 0x800 + ln * 0x100 + 4 * dw }
pub const fn tx_grp(phy: u32, dw: u32) -> u32 { base(phy) + 0x680 + 4 * dw }
pub const fn tx_ln(phy: u32, ln: u32, dw: u32) -> u32 { base(phy) + 0x880 + ln * 0x100 + 4 * dw }

pub const SUS_CLOCK_CONFIG: u32 = 3;
pub const EDP4K2K_MODE_OVRD: u32 = 3 << 2; // CL_DW10 bits 3:2 (enable + optimized)
pub const COMMON_KEEPER_EN: u32 = 1 << 26;
pub const SWING_SEL_UPPER: u32 = 1 << 15;
pub const SWING_SEL_LOWER_SHIFT: u32 = 11;
pub const SWING_SEL_LOWER_MASK: u32 = 7 << 11;
pub const RCOMP_SCALAR_MASK: u32 = 0xff;
pub const LOADGEN_SELECT: u32 = 1 << 31;
pub const POST_CURSOR_1_SHIFT: u32 = 12;
pub const POST_CURSOR_2_SHIFT: u32 = 6;
pub const CURSOR_MASK: u32 = 0x3f;
pub const TX_TRAINING_EN: u32 = 1 << 31;
pub const TAP2_DISABLE: u32 = 1 << 30;
pub const TAP3_DISABLE: u32 = 1 << 29;
pub const CURSOR_PROGRAM: u32 = 1 << 26;
pub const COEFF_POLARITY: u32 = 1 << 25;
pub const SCALING_MODE_SEL_MASK: u32 = 7 << 18;
pub const RTERM_SELECT_MASK: u32 = 7 << 3;
pub const N_SCALAR_MASK: u32 = 0x7f << 24;

/// One swing/de-emphasis setting: DW2 swing select, DW7 N scalar, DW4
/// cursor coefficient and post-cursors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trans { pub swing: u8, pub n_scalar: u8, pub cursor: u8, pub post1: u8, pub post2: u8 }
const fn t(swing: u8, n_scalar: u8, cursor: u8, post2: u8, post1: u8) -> Trans { Trans { swing, n_scalar, cursor, post1, post2 } }

/// Rows in DP order: (swing 0, pre 0..3), (1, 0..2), (2, 0..1), (3, 0).
pub const DP_HBR: [Trans; 10] = [
    t(0xA, 0x32, 0x3F, 0, 0x00), t(0xA, 0x4F, 0x37, 0, 0x08), t(0xC, 0x71, 0x2F, 0, 0x10), t(0x6, 0x7D, 0x2B, 0, 0x14),
    t(0xA, 0x4C, 0x3F, 0, 0x00), t(0xC, 0x73, 0x34, 0, 0x0B), t(0x6, 0x7F, 0x2F, 0, 0x10),
    t(0xC, 0x6C, 0x3C, 0, 0x03), t(0x6, 0x7F, 0x35, 0, 0x0A), t(0x6, 0x7F, 0x3F, 0, 0x00),
];
pub const DP_HBR2: [Trans; 10] = [
    t(0xA, 0x35, 0x3F, 0, 0x00), t(0xA, 0x4F, 0x37, 0, 0x08), t(0xC, 0x63, 0x2F, 0, 0x10), t(0x6, 0x7F, 0x2B, 0, 0x14),
    t(0xA, 0x47, 0x3F, 0, 0x00), t(0xC, 0x63, 0x34, 0, 0x0B), t(0x6, 0x7F, 0x2F, 0, 0x10),
    t(0xC, 0x61, 0x3C, 0, 0x03), t(0x6, 0x7B, 0x35, 0, 0x0A), t(0x6, 0x7F, 0x3F, 0, 0x00),
];
/// eDP above HBR2 (HBR3) and DP HBR2 on older parts share this table.
pub const EDP_HBR3: [Trans; 10] = [
    t(0xA, 0x35, 0x3F, 0, 0x00), t(0xA, 0x4F, 0x37, 0, 0x08), t(0xC, 0x71, 0x2F, 0, 0x10), t(0x6, 0x7F, 0x2B, 0, 0x14),
    t(0xA, 0x4C, 0x3F, 0, 0x00), t(0xC, 0x73, 0x34, 0, 0x0B), t(0x6, 0x7F, 0x2F, 0, 0x10),
    t(0xC, 0x6C, 0x3C, 0, 0x03), t(0x6, 0x7F, 0x35, 0, 0x0A), t(0x6, 0x7F, 0x3F, 0, 0x00),
];

/// The table for a link rate (kHz). eDP uses the DP tables up to HBR2 (the
/// PRM's default when the VBT asks for no low-swing table).
pub fn table(rate_khz: u32, edp: bool) -> &'static [Trans; 10] {
    if edp && rate_khz > 540_000 { &EDP_HBR3 } else if rate_khz > 270_000 { &DP_HBR2 } else { &DP_HBR }
}

/// Row of a (swing, pre-emphasis) pair, clamped to what DP allows (sum <= 3).
pub fn row(swing: u8, pre: u8) -> usize {
    let s = swing.min(3);
    let p = pre.min(3 - s);
    [0, 4, 7, 9][s as usize] + p as usize
}

/// Program per-lane levels (`levels[ln]` = (swing, pre)) on combo PHY `phy`,
/// as the PRM orders it: keeper, loadgen, SUS clock, training off, swing,
/// training on (the last write latches the new levels).
pub fn set_levels(m: &mut impl Mmio, phy: u32, rate_khz: u32, lanes: u32, edp: bool, levels: &[(u8, u8); 4]) {
    let tbl = table(rate_khz, edp);
    let v = m.read(pcs_ln(phy, 0, 1));
    m.write(pcs_grp(phy, 1), v | COMMON_KEEPER_EN);
    for ln in 0..4 {
        let load = if rate_khz > 600_000 { false } else if lanes == 4 { ln >= 1 } else { ln == 1 || ln == 2 };
        m.modify(tx_ln(phy, ln, 4), LOADGEN_SELECT, if load { LOADGEN_SELECT } else { 0 });
    }
    m.modify(cl_dw(phy, 5), 0, SUS_CLOCK_CONFIG);
    let dw5 = m.read(tx_ln(phy, 0, 5));
    m.write(tx_grp(phy, 5), dw5 & !TX_TRAINING_EN);
    if edp { m.modify(cl_dw(phy, 10), EDP4K2K_MODE_OVRD, 0); }
    let dw5 = m.read(tx_ln(phy, 0, 5)) & !(SCALING_MODE_SEL_MASK | RTERM_SELECT_MASK | COEFF_POLARITY | CURSOR_PROGRAM | TAP2_DISABLE | TAP3_DISABLE);
    m.write(tx_grp(phy, 5), dw5 | 2 << 18 | 6 << 3 | TAP3_DISABLE);
    for ln in 0..4u32 {
        let e = tbl[row(levels[ln as usize].0, levels[ln as usize].1)];
        let s = e.swing as u32;
        m.modify(tx_ln(phy, ln, 2), SWING_SEL_UPPER | SWING_SEL_LOWER_MASK | RCOMP_SCALAR_MASK,
            (s >> 3) * SWING_SEL_UPPER | (s & 7) << SWING_SEL_LOWER_SHIFT | 0x98);
    }
    for ln in 0..4u32 {
        let e = tbl[row(levels[ln as usize].0, levels[ln as usize].1)];
        m.modify(tx_ln(phy, ln, 4), CURSOR_MASK << POST_CURSOR_1_SHIFT | CURSOR_MASK << POST_CURSOR_2_SHIFT | CURSOR_MASK,
            (e.post1 as u32) << POST_CURSOR_1_SHIFT | (e.post2 as u32) << POST_CURSOR_2_SHIFT | e.cursor as u32);
    }
    for ln in 0..4u32 {
        let e = tbl[row(levels[ln as usize].0, levels[ln as usize].1)];
        m.modify(tx_ln(phy, ln, 7), N_SCALAR_MASK, (e.n_scalar as u32) << 24);
    }
    let dw5 = m.read(tx_ln(phy, 0, 5));
    m.write(tx_grp(phy, 5), dw5 | TX_TRAINING_EN);
}
