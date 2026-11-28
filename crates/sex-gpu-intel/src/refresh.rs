// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Seamless refresh-rate changes on a running DisplayPort/eDP pipe.
//!
//! On a DP link the transcoder is clocked by the link, and the pixel rate the
//! sink rebuilds is set only by the data and link M/N ratios. When two modes
//! share a raster (same h/v counts and sync, different pixel clock), moving
//! between them is four register writes: no pipe off, no link training, no
//! blank frame. Gen9+ display engines double-buffer M1/N1, latching them at
//! the next vblank (PRM Vol 2c TRANS_DATA_M1; i915 relies on the same thing
//! for seamless DRRS on Gen12, see docs/BRIDGE_LEDGER.md table 2).
//!
//! N stays what firmware chose; M scales with the pixel clock, so the link
//! rate and lane count never need to be known. The data ratio M/N is the
//! share of the link the stream uses: it must stay below one.

use crate::regs::*;
use crate::timing::Timing;
use crate::watermarks::{self as wm, Which, Wm};
use crate::{Error, Mmio};

pub const M_MASK: u32 = 0xff_ffff;
pub const TU_SHIFT: u32 = 25;
pub const TU_MASK: u32 = 0x3f << TU_SHIFT;

/// Transcoder M/N set 1, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MN { pub tu: u8, pub data_m: u32, pub data_n: u32, pub link_m: u32, pub link_n: u32 }

impl MN {
    /// Share of the link's payload capacity the stream uses, in parts per 10,000.
    pub fn link_use_bp(&self) -> u32 {
        if self.data_n == 0 { return u32::MAX; }
        (self.data_m as u64 * 10_000 / self.data_n as u64) as u32
    }
    /// The same link carrying a stream at `to_khz` instead of `from_khz`.
    /// None when the result would not fit the link (with 0.5 % margin for
    /// the main-stream attribute and blanking symbols) or into 24 bits.
    pub fn scaled(&self, from_khz: u32, to_khz: u32) -> Option<MN> {
        if from_khz == 0 || to_khz == 0 || self.data_n == 0 || self.link_n == 0 { return None; }
        let s = |v: u32| -> Option<u32> {
            let r = (v as u64 * to_khz as u64 + from_khz as u64 / 2) / from_khz as u64;
            (r > 0 && r <= M_MASK as u64).then_some(r as u32)
        };
        let out = MN { data_m: s(self.data_m)?, link_m: s(self.link_m)?, ..*self };
        (out.link_use_bp() <= 9_950).then_some(out)
    }
}

pub fn read(m: &mut impl Mmio, t: u32) -> MN {
    let dm = m.read(trans(t, DATA_M1));
    MN {
        tu: (((dm & TU_MASK) >> TU_SHIFT) + 1) as u8,
        data_m: dm & M_MASK,
        data_n: m.read(trans(t, DATA_N1)) & M_MASK,
        link_m: m.read(trans(t, LINK_M1)) & M_MASK,
        link_n: m.read(trans(t, LINK_N1)) & M_MASK,
    }
}

/// Pixel clock (kHz) the transcoder is pacing, from its link M/N and the
/// link symbol clock (link rate / 10 for 8b/10b DP).
pub fn pixel_khz(mn: &MN, link_symbol_khz: u32) -> u32 {
    if mn.link_n == 0 { 0 } else { (mn.link_m as u64 * link_symbol_khz as u64 / mn.link_n as u64) as u32 }
}

/// Write the four registers inside one frame. They latch together at the
/// next vblank, so the write waits until the pipe is drawing the first half
/// of its active area: never across a vblank.
pub fn write(m: &mut impl Mmio, t: u32, mn: &MN, vactive: u32, budget: u32) -> Result<(), Error> {
    let mut ok = false;
    for _ in 0..budget.max(1) {
        let line = crate::plane::scanline(m, t);
        if line < vactive / 2 { ok = true; break; }
    }
    if !ok { return Err(Error::Timeout(pipe(t, PIPEDSL))); }
    let keep = m.read(trans(t, DATA_M1)) & !(TU_MASK | M_MASK);
    m.write(trans(t, DATA_M1), keep | ((mn.tu.max(1) as u32 - 1) << TU_SHIFT) & TU_MASK | mn.data_m);
    m.write(trans(t, DATA_N1), mn.data_n);
    m.write(trans(t, LINK_M1), mn.link_m);
    m.write(trans(t, LINK_N1), mn.link_n);
    Ok(())
}

/// Why a refresh change was refused. Each case leaves the pipe untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a DP/eDP SST pipe (HDMI pixel clocks come from the PLL).
    NotDp,
    /// The target mode has a different raster; it needs a full modeset.
    Raster,
    /// The link cannot carry the faster stream (it was trained for less).
    Link,
    /// The faster fetch would not fit the plane's DBUF share at level 0.
    Watermarks,
    /// Pixel rate above what CDCLK can feed (two pixels per CDCLK on Gen11+).
    Cdclk,
    Device(Error),
}
impl Refusal {
    pub fn name(&self) -> &'static str {
        match self {
            Refusal::NotDp => "not_dp", Refusal::Raster => "different_raster", Refusal::Link => "link_bandwidth",
            Refusal::Watermarks => "watermarks", Refusal::Cdclk => "cdclk", Refusal::Device(_) => "device",
        }
    }
}

/// CDCLK in kHz from CDCLK_CTL's decimal field (PRM: (freq - 1 MHz) / 0.5 MHz).
pub fn cdclk_khz(m: &mut impl Mmio) -> u32 { (m.read(CDCLK_CTL) & 0x7ff) * 500 + 1000 }

/// What firmware left: its pixel clock and the watermarks it sized for it.
/// Taken once, before SexOS changes anything, so later plans scale from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boot { pub pipe: u32, pub khz: u32, pub width: u32, pub primary: Wm, pub cursor: Wm }
impl Boot {
    pub fn read(m: &mut impl Mmio, p: u32, khz: u32, width: u32) -> Boot {
        Boot { pipe: p, khz, width, primary: wm::read(m, p, Which::Plane(1)), cursor: wm::read(m, p, Which::Cursor) }
    }
}

/// A planned switch: what is running, where to go, the new M/N, and the
/// watermarks to latch first when the new rate is faster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub pipe: u32,
    pub from: Timing,
    pub to: Timing,
    pub before: MN,
    pub after: MN,
    /// Some when the plane and cursor need deeper watermarks before the switch.
    pub wm: Option<(Wm, Wm)>,
}

/// Check a switch of pipe `p` from `running` to `target`.
pub fn plan(m: &mut impl Mmio, p: u32, running: &Timing, target: &Timing, boot: &Boot) -> Result<Plan, Refusal> {
    let func = m.read(trans(p, DDI_FUNC_CTL));
    if func & FUNC_ENABLE == 0 || (func >> FUNC_MODE_SHIFT) & 7 != FUNC_MODE_DP_SST { return Err(Refusal::NotDp); }
    if !running.same_raster(target) { return Err(Refusal::Raster); }
    if target.clock_khz > 2 * cdclk_khz(m) { return Err(Refusal::Cdclk); }
    let before = read(m, p);
    let after = before.scaled(running.clock_khz, target.clock_khz).ok_or(Refusal::Link)?;
    let wm = if target.clock_khz > running.clock_khz && target.clock_khz > boot.khz {
        let primary = wm::scale(&boot.primary, boot.khz, target.clock_khz, boot.width).ok_or(Refusal::Watermarks)?;
        let cursor = wm::scale(&boot.cursor, boot.khz, target.clock_khz, 256).ok_or(Refusal::Watermarks)?;
        Some((primary, cursor))
    } else { None };
    Ok(Plan { pipe: p, from: *running, to: *target, before, after, wm })
}

/// Carry the plan out: deeper watermarks first (latched by re-arming the
/// plane and cursor with their current surfaces, then one vblank), then M/N.
pub fn apply(m: &mut impl Mmio, plan: &Plan, budget: u32) -> Result<(), Refusal> {
    let p = plan.pipe;
    if let Some((primary, cursor)) = &plan.wm {
        wm::write(m, p, Which::Plane(1), primary);
        wm::write(m, p, Which::Cursor, cursor);
        let surf = m.read(plane(p, 1, PLANE_SURF));
        m.write(plane(p, 1, PLANE_SURF), surf);
        let base = m.read(cursor_reg(p));
        m.write(cursor_reg(p), base);
        crate::plane::wait_vblank(m, p, budget).map_err(Refusal::Device)?;
    }
    write(m, p, &plan.after, plan.to.v[0] as u32, budget).map_err(Refusal::Device)
}

fn cursor_reg(p: u32) -> u32 { cursor(p, CUR_BASE) }
