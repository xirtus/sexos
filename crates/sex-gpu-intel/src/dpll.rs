// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Combo PHY display PLLs (DPLL0/DPLL1) on Gen12: the clocks that drive DDI
//! A and B, which carry the eDP panel and HDMI/DP on combo ports.
//!
//! Readout tells SexOS the link rate firmware trained (the receipt and the
//! refresh logic use it); the fixed DP table and the enable sequence are what
//! a full modeset programs. Register layout from the TGL PRM Vol 2c
//! (DPLL_CFGCR0/1, DPLL_ENABLE, DPCLKA_CFGCR0); the DP divider table is the
//! one the PRM lists for 19.2/38.4 MHz and 24 MHz references, cross-checked
//! with the MIT-licensed i915 `icl_dp_combo_pll_*_values` (BRIDGE_LEDGER t2).

use crate::{wait_for, Error, Mmio};

pub const DSSM: u32 = 0x51004;
pub const fn enable_reg(pll: u32) -> u32 { 0x46010 + pll * 4 }
pub const fn cfgcr0(pll: u32) -> u32 { 0x164284 + pll * 8 }
pub const fn cfgcr1(pll: u32) -> u32 { 0x164288 + pll * 8 }
pub const DPCLKA_CFGCR0: u32 = 0x164280;

pub const PLL_ENABLE: u32 = 1 << 31;
pub const PLL_LOCK: u32 = 1 << 30;
pub const PLL_POWER_ENABLE: u32 = 1 << 27;
pub const PLL_POWER_STATE: u32 = 1 << 26;

pub const DCO_FRACTION_SHIFT: u32 = 10;
pub const DCO_FRACTION_MASK: u32 = 0x7fff << 10;
pub const DCO_INTEGER_MASK: u32 = 0x3ff;
pub const QDIV_RATIO_SHIFT: u32 = 10;
pub const QDIV_RATIO_MASK: u32 = 0xff << 10;
pub const QDIV_MODE: u32 = 1 << 9;
pub const KDIV_SHIFT: u32 = 6;
pub const KDIV_MASK: u32 = 7 << 6;
pub const PDIV_SHIFT: u32 = 2;
pub const PDIV_MASK: u32 = 0xf << 2;

/// Display reference clock in kHz (DSSM bits 31:29).
pub fn refclk_khz(m: &mut impl Mmio) -> u32 {
    match (m.read(DSSM) >> 29) & 7 { 0 => 24_000, 1 => 19_200, _ => 38_400 }
}

/// Raw divider fields as the registers hold them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params { pub dco_integer: u32, pub dco_fraction: u32, pub pdiv: u32, pub kdiv: u32, pub qdiv_mode: u32, pub qdiv_ratio: u32 }

const fn p(i: u32, f: u32, pdiv: u32, kdiv: u32, qm: u32, qr: u32) -> Params {
    Params { dco_integer: i, dco_fraction: f, pdiv, kdiv, qdiv_mode: qm, qdiv_ratio: qr }
}
/// DP link rates (kHz symbol clock = link rate / 10 bits) and their dividers.
pub const DP_RATES: [u32; 8] = [540_000, 270_000, 162_000, 324_000, 216_000, 432_000, 648_000, 810_000];
const DP_19_2: [Params; 8] = [
    p(0x1a5, 0x7000, 0x2, 1, 0, 0), p(0x1a5, 0x7000, 0x2, 2, 0, 0), p(0x1a5, 0x7000, 0x4, 2, 0, 0), p(0x1a5, 0x7000, 0x4, 1, 0, 0),
    p(0x1c2, 0x0000, 0x1, 2, 1, 2), p(0x1c2, 0x0000, 0x1, 2, 0, 0), p(0x1fa, 0x2000, 0x2, 1, 0, 0), p(0x1a5, 0x7000, 0x1, 1, 0, 0),
];
const DP_24: [Params; 8] = [
    p(0x151, 0x4000, 0x2, 1, 0, 0), p(0x151, 0x4000, 0x2, 2, 0, 0), p(0x151, 0x4000, 0x4, 2, 0, 0), p(0x151, 0x4000, 0x4, 1, 0, 0),
    p(0x168, 0x0000, 0x1, 2, 1, 2), p(0x168, 0x0000, 0x1, 2, 0, 0), p(0x195, 0x0000, 0x2, 1, 0, 0), p(0x151, 0x4000, 0x1, 1, 0, 0),
];

/// Divider values for a DP link rate (kHz, e.g. 270000 = HBR).
pub fn dp_params(refclk_khz: u32, rate_khz: u32) -> Option<Params> {
    let t = if refclk_khz == 24_000 { &DP_24 } else { &DP_19_2 };
    DP_RATES.iter().position(|&r| r == rate_khz).map(|i| t[i])
}

/// The two config registers for `p`. With a 38.4 MHz reference the DCO
/// fraction is halved (the PLL divides the reference by two on Gen12).
pub fn encode(refclk_khz: u32, p: &Params) -> (u32, u32) {
    let frac = if refclk_khz == 38_400 { (p.dco_fraction + 1) / 2 } else { p.dco_fraction };
    let c0 = (frac << DCO_FRACTION_SHIFT) & DCO_FRACTION_MASK | p.dco_integer & DCO_INTEGER_MASK;
    // CFSELOVRD = 0: normal crystal (TGL); kdiv is a one-hot 1/2/3 field.
    let kdiv = match p.kdiv { 1 => 1, 2 => 2, 3 => 4, k => k };
    let c1 = p.qdiv_ratio << QDIV_RATIO_SHIFT & QDIV_RATIO_MASK | if p.qdiv_mode != 0 { QDIV_MODE } else { 0 }
        | kdiv << KDIV_SHIFT & KDIV_MASK | p.pdiv << PDIV_SHIFT & PDIV_MASK;
    (c0, c1)
}

/// Port clock (kHz) a configured PLL produces: DCO / (p * q * k * 5).
pub fn port_clock_khz(refclk_khz: u32, c0: u32, c1: u32) -> u32 {
    let refk = if refclk_khz == 38_400 { 19_200 } else { refclk_khz } as u64;
    let p0 = match (c1 & PDIV_MASK) >> PDIV_SHIFT { 1 => 2, 2 => 3, 4 => 5, 8 => 7, _ => 0 };
    let p2 = match (c1 & KDIV_MASK) >> KDIV_SHIFT { 1 => 1, 2 => 2, 4 => 3, _ => 0 };
    let p1 = if c1 & QDIV_MODE != 0 { (c1 & QDIV_RATIO_MASK) >> QDIV_RATIO_SHIFT } else { 1 };
    let mut frac = ((c0 & DCO_FRACTION_MASK) >> DCO_FRACTION_SHIFT) as u64;
    if refclk_khz == 38_400 { frac *= 2; }
    let dco = (c0 & DCO_INTEGER_MASK) as u64 * refk + frac * refk / 0x8000;
    let div = p0 as u64 * p1 as u64 * p2 as u64 * 5;
    if div == 0 { 0 } else { (dco / div) as u32 }
}

/// Combo PHY index of a DDI port (A = 0, B = 1); Type-C ports use their own PLLs.
pub fn combo_phy(port: u32) -> Option<u32> { (port <= 1).then_some(port) }
fn clk_off_bit(phy: u32) -> u32 { 1 << (10 + phy) }
fn clk_sel_shift(phy: u32) -> u32 { phy * 2 }

/// Which DPLL feeds a combo port, if its DDI clock is on.
pub fn port_pll(m: &mut impl Mmio, port: u32) -> Option<u32> {
    let phy = combo_phy(port)?;
    let v = m.read(DPCLKA_CFGCR0);
    (v & clk_off_bit(phy) == 0).then_some((v >> clk_sel_shift(phy)) & 3)
}

/// The link rate (kHz) a running combo port uses, from its PLL.
pub fn port_link_khz(m: &mut impl Mmio, port: u32) -> Option<u32> {
    let pll = port_pll(m, port)?;
    if m.read(enable_reg(pll)) & PLL_LOCK == 0 { return None; }
    let r = refclk_khz(m);
    Some(port_clock_khz(r, m.read(cfgcr0(pll)), m.read(cfgcr1(pll))))
}

/// PRM enable sequence: power the PLL, program the dividers, enable, lock.
pub fn enable(m: &mut impl Mmio, pll: u32, c0: u32, c1: u32, budget: u32) -> Result<(), Error> {
    m.modify(enable_reg(pll), 0, PLL_POWER_ENABLE);
    wait_for(m, enable_reg(pll), PLL_POWER_STATE, PLL_POWER_STATE, budget)?;
    m.write(cfgcr0(pll), c0);
    m.write(cfgcr1(pll), c1);
    let _ = m.read(cfgcr1(pll)); // posting read
    m.modify(enable_reg(pll), 0, PLL_ENABLE);
    wait_for(m, enable_reg(pll), PLL_LOCK, PLL_LOCK, budget).map(|_| ())
}

pub fn disable(m: &mut impl Mmio, pll: u32, budget: u32) -> Result<(), Error> {
    m.modify(enable_reg(pll), PLL_ENABLE, 0);
    wait_for(m, enable_reg(pll), PLL_LOCK, 0, budget)?;
    m.modify(enable_reg(pll), PLL_POWER_ENABLE, 0);
    wait_for(m, enable_reg(pll), PLL_POWER_STATE, 0, budget).map(|_| ())
}

/// Route `pll` to combo port `port` and ungate its DDI clock (select first,
/// then clear the gate, as the PRM orders it).
pub fn map_port(m: &mut impl Mmio, port: u32, pll: u32) -> Result<(), Error> {
    let phy = combo_phy(port).ok_or(Error::BadArgument)?;
    m.modify(DPCLKA_CFGCR0, 3 << clk_sel_shift(phy), (pll & 3) << clk_sel_shift(phy));
    m.modify(DPCLKA_CFGCR0, clk_off_bit(phy), 0);
    Ok(())
}

pub fn gate_port(m: &mut impl Mmio, port: u32) {
    if let Some(phy) = combo_phy(port) { m.modify(DPCLKA_CFGCR0, 0, clk_off_bit(phy)); }
}
