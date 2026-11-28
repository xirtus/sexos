// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Full modeset of a DisplayPort/eDP SST output on a combo PHY port (DDI A/B):
//! pick a link, compute M/N, and run the PRM's "Enable sequence for
//! DisplayPort" / "Disable sequence" (TGL PRM Vol 12, DDI and transcoder
//! chapters; i915's tgl_ddi_pre_enable_dp step numbering used as the
//! cross-check, BRIDGE_LEDGER t2):
//!
//!  enable: PLL on and mapped -> transcoder clock -> TRANS_DDI_FUNC_CTL ->
//!          DP_TP_CTL pattern 1 + DDI_BUF_CTL -> lanes powered -> swing ->
//!          link training -> DP_TP_CTL idle -> M/N, timings, PIPESRC ->
//!          TRANSCONF on -> DP_TP_CTL normal, sink pattern off.
//!  disable: TRANSCONF off -> FUNC_CTL off -> (sink D3 unless eDP stays on)
//!          -> DDI_BUF_CTL off, wait idle -> DP_TP_CTL off -> no transcoder
//!          clock -> DDI clock gated -> PLL off.
//!
//! Planes and watermarks are the caller's: they latch per plane and are
//! programmed once the pipe runs (see `plane` and `watermarks`).

use crate::refresh::{self, MN};
use crate::regs::*;
use crate::timing::Timing;
use crate::training::{self, Config, Failure, Pattern, Source, Trained};
use crate::{aux, dpll, phy, wait_for, Error, Mmio};

pub const fn trans_clk_sel(t: u32) -> u32 { 0x46140 + t * 4 }
pub const fn dp_tp_ctl(t: u32) -> u32 { trans(t, 0x540) }
pub const fn dp_tp_status(t: u32) -> u32 { trans(t, 0x544) }
pub const fn pipe_misc(p: u32) -> u32 { pipe(p, 0x30) }
pub const TP_ENABLE: u32 = 1 << 31;
pub const TP_ENHANCED_FRAME: u32 = 1 << 18;
pub const TP_TRAIN_MASK: u32 = 7 << 8;
pub const TP_PAT1: u32 = 0 << 8;
pub const TP_PAT2: u32 = 1 << 8;
pub const TP_IDLE: u32 = 2 << 8;
pub const TP_NORMAL: u32 = 3 << 8;
pub const TP_PAT3: u32 = 4 << 8;
pub const TP_PAT4: u32 = 5 << 8;
pub const BUF_PORT_WIDTH_SHIFT: u32 = 1;
pub const BUF_PORT_WIDTH_MASK: u32 = 7 << 1;
pub const PIPE_MISC_BPC_MASK: u32 = 7 << 5;
pub const FUNC_WIDTH_MASK: u32 = 7 << 1;
pub const CL_DW10_PWR_DOWN_MASK: u32 = 0xf << 4;

/// Bits per pixel the link carries for an RGB 8 bpc stream.
pub const BPP_8BPC: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link { pub rate_khz: u32, pub lanes: u8 }

/// The cheapest link that carries `pixel_khz` at `bpp` (DP 8b/10b: each lane
/// moves 8 data bits per symbol clock), keeping 0.6 % for the MSA and
/// blanking symbols. Fewest lanes at the lowest rate wins; eDP panels may
/// need their full lane count, which `min_lanes` enforces.
pub fn choose_link(pixel_khz: u32, bpp: u32, rates: &[u32], max_lanes: u8, min_lanes: u8) -> Option<Link> {
    let need = pixel_khz as u64 * bpp as u64 * 1006 / 1000;
    let mut best: Option<(u64, Link)> = None;
    for lanes in [1u8, 2, 4] {
        if lanes > max_lanes || lanes < min_lanes { continue; }
        for &r in rates {
            let have = r as u64 * lanes as u64 * 8;
            if have < need { continue; }
            if best.map_or(true, |(h, _)| have < h) { best = Some((have, Link { rate_khz: r, lanes })); }
        }
    }
    best.map(|b| b.1)
}

/// Data and link M/N for a stream on a link, with the fixed N values PC
/// firmware uses (0x800000 data, 0x80000 link) and TU 64.
pub fn mn(pixel_khz: u32, bpp: u32, link: &Link) -> MN {
    let data_n = 0x80_0000u64;
    let link_n = 0x8_0000u64;
    let data_m = pixel_khz as u64 * bpp as u64 * data_n / (link.rate_khz as u64 * link.lanes as u64 * 8);
    let link_m = pixel_khz as u64 * link_n / link.rate_khz as u64;
    MN { tu: 64, data_m: data_m as u32, data_n: data_n as u32, link_m: link_m as u32, link_n: link_n as u32 }
}

/// Everything one output needs to light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan { pub pipe: u32, pub port: u32, pub pll: u32, pub timing: Timing, pub link: Link, pub edp: bool, pub tps3: bool, pub tps4: bool, pub rate_index: Option<u8> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Pll, Buffer, Training(Failure), Pipe, Sink }
impl Step {
    pub fn name(&self) -> &'static str {
        match self { Step::Pll => "pll", Step::Buffer => "ddi_buffer", Step::Training(Failure::ClockRecovery) => "clock_recovery",
            Step::Training(Failure::Equalisation) => "equalisation", Step::Training(Failure::Aux(_)) => "aux", Step::Pipe => "pipe", Step::Sink => "sink" }
    }
}

/// The source side of training on a combo PHY port.
struct Combo<'a, M: Mmio, D: FnMut(u32)> { m: &'a mut M, plan: &'a Plan, delay: D }
impl<'a, M: Mmio, D: FnMut(u32)> Source for Combo<'a, M, D> {
    fn set_pattern(&mut self, p: Pattern) {
        let bits = match p { Pattern::Off => TP_IDLE, Pattern::Tps1 => TP_PAT1, Pattern::Tps2 => TP_PAT2, Pattern::Tps3 => TP_PAT3, Pattern::Tps4 => TP_PAT4 };
        self.m.modify(dp_tp_ctl(self.plan.pipe), TP_TRAIN_MASK, bits);
    }
    fn set_levels(&mut self, l: &[(u8, u8); 4]) {
        phy::set_levels(self.m, self.plan.port, self.plan.link.rate_khz, self.plan.link.lanes as u32, self.plan.edp, l);
    }
    fn dpcd_read(&mut self, a: u32, b: &mut [u8]) -> Result<(), Error> { aux::dpcd_read(self.m, self.plan.port, a, b) }
    fn dpcd_write(&mut self, a: u32, d: &[u8]) -> Result<(), Error> { aux::dpcd_write(self.m, self.plan.port, a, d) }
    fn delay_us(&mut self, us: u32) { (self.delay)(us) }
}

fn pair(a: u16, b: u16) -> u32 { (b as u32 - 1) << 16 | (a as u32 - 1) }

/// Turn the pipe, transcoder, DDI and PLL off. `keep_sink` leaves the sink
/// awake (an eDP panel about to be relit keeps its power and backlight).
pub fn disable(m: &mut impl Mmio, pipe_n: u32, port: u32, pll: Option<u32>, keep_sink: bool, budget: u32) -> Result<(), Error> {
    m.modify(plane(pipe_n, 1, PLANE_CTL), PLANE_ENABLE, 0);
    let surf = m.read(plane(pipe_n, 1, PLANE_SURF));
    m.write(plane(pipe_n, 1, PLANE_SURF), surf);
    m.modify(pipe(pipe_n, CONF), CONF_ENABLE, 0);
    wait_for(m, pipe(pipe_n, CONF), CONF_STATE, 0, budget)?;
    m.modify(trans(pipe_n, DDI_FUNC_CTL), FUNC_ENABLE, 0);
    if !keep_sink { let _ = aux::dpcd_write(m, port, aux::dpcd::SET_POWER, &[2]); }
    m.modify(ddi_buf_ctl(port), DDI_BUF_ENABLE, 0);
    let idle = wait_for(m, ddi_buf_ctl(port), DDI_BUF_IDLE, DDI_BUF_IDLE, budget);
    m.modify(dp_tp_ctl(pipe_n), TP_ENABLE, 0);
    m.write(trans_clk_sel(pipe_n), 0);
    dpll::gate_port(m, port);
    if let Some(pll) = pll { dpll::disable(m, pll, budget)?; }
    idle.map(|_| ())
}

/// Light an output per `plan`. Returns the training result. On failure the
/// caller runs [`disable`] (or falls back to the previous mode).
pub fn enable(m: &mut impl Mmio, plan: &Plan, budget: u32, mut delay: impl FnMut(u32)) -> Result<Trained, Step> {
    let (p, port, t) = (plan.pipe, plan.port, &plan.timing);
    // 4. Port PLL: program, lock, route to the DDI.
    let refk = dpll::refclk_khz(m);
    let params = dpll::dp_params(refk, plan.link.rate_khz).ok_or(Step::Pll)?;
    let (c0, c1) = dpll::encode(refk, &params);
    dpll::enable(m, plan.pll, c0, c1, budget).map_err(|_| Step::Pll)?;
    dpll::map_port(m, port, plan.pll).map_err(|_| Step::Pll)?;
    // 7.a transcoder clock from the port; 7.b DDI select and mode.
    m.write(trans_clk_sel(p), (port + 1) << 28);
    let lanes = plan.link.lanes as u32;
    let mut func = FUNC_ENABLE | (port + 1) << FUNC_PORT_SHIFT | FUNC_MODE_DP_SST << FUNC_MODE_SHIFT | (lanes - 1) << 1;
    if t.flags & 1 != 0 { func |= FUNC_PHSYNC; }
    if t.flags & 2 != 0 { func |= FUNC_PVSYNC; }
    m.write(trans(p, DDI_FUNC_CTL), func);
    // 7.c DP_TP_CTL with pattern 1, then the DDI buffer on.
    m.write(dp_tp_ctl(p), TP_ENABLE | TP_ENHANCED_FRAME | TP_PAT1);
    let _ = m.read(dp_tp_ctl(p));
    let width = if lanes == 4 { 3 } else { lanes - 1 };
    m.write(ddi_buf_ctl(port), DDI_BUF_ENABLE | width << BUF_PORT_WIDTH_SHIFT);
    // TGL combo PHY: the buffer leaves idle within 1 ms.
    let mut active = false;
    for _ in 0..10 { if m.read(ddi_buf_ctl(port)) & DDI_BUF_IDLE == 0 { active = true; break; } delay(100); }
    if !active { return Err(Step::Buffer); }
    // 7.f power up the used lanes (no lane reversal).
    let down = match lanes { 1 => 0xe, 2 => 0xc, _ => 0 } << 4;
    m.modify(phy::cl_dw(port, 10), CL_DW10_PWR_DOWN_MASK, down);
    // Wake the sink (DP SET_POWER D0) and train.
    let _ = aux::dpcd_write(m, port, aux::dpcd::SET_POWER, &[1]);
    delay(1000);
    let cfg = Config { rate_khz: plan.link.rate_khz, lanes: plan.link.lanes, rate_index: plan.rate_index, tps3: plan.tps3, tps4: plan.tps4 };
    let trained = {
        let mut src = Combo { m: &mut *m, plan, delay: &mut delay };
        training::train(&mut src, &cfg).map_err(Step::Training)?
    };
    // 7.j idle pattern while the transcoder comes up.
    m.modify(dp_tp_ctl(p), TP_TRAIN_MASK, TP_IDLE);
    // Transcoder: M/N, timings, source size, bpc.
    let v = mn(t.clock_khz, BPP_8BPC, &plan.link);
    refresh::write(m, p, &v, u32::MAX, 1).map_err(|_| Step::Pipe)?;
    m.write(trans(p, HTOTAL), pair(t.h[0], t.h[3]));
    m.write(trans(p, HBLANK), pair(t.h[0], t.h[3]));
    m.write(trans(p, HSYNC), pair(t.h[1], t.h[2]));
    m.write(trans(p, VTOTAL), pair(t.v[0], t.v[3]));
    m.write(trans(p, VBLANK), pair(t.v[0], t.v[3]));
    m.write(trans(p, VSYNC), pair(t.v[1], t.v[2]));
    m.write(trans(p, PIPESRC), (t.h[0] as u32 - 1) << 16 | (t.v[0] as u32 - 1));
    m.modify(pipe_misc(p), PIPE_MISC_BPC_MASK, 0);
    m.modify(pipe(p, CONF), 0, CONF_ENABLE);
    wait_for(m, pipe(p, CONF), CONF_STATE, CONF_STATE, budget).map_err(|_| Step::Pipe)?;
    // 7.k pixels, and the sink out of training.
    m.modify(dp_tp_ctl(p), TP_TRAIN_MASK, TP_NORMAL);
    training::finish(&mut Combo { m, plan, delay: &mut delay }).map_err(|_| Step::Sink)?;
    Ok(trained)
}
