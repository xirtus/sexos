// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DisplayPort link training (VESA DP 1.4a §3.5.1.2): clock recovery with
//! TPS1, then channel equalisation with the best pattern the sink supports,
//! adjusting each lane's voltage swing and pre-emphasis as the sink asks.
//!
//! The procedure is the standard's; the source side is a [`Source`] so the
//! same code drives a combo PHY, a Type-C PHY or a test double.

use crate::Error;

pub mod dpcd {
    pub const MAX_DOWNSPREAD: u32 = 0x003;
    pub const TRAINING_AUX_RD_INTERVAL: u32 = 0x00e;
    pub const LINK_BW_SET: u32 = 0x100;
    pub const LANE_COUNT_SET: u32 = 0x101;
    pub const TRAINING_PATTERN_SET: u32 = 0x102;
    pub const TRAINING_LANE0_SET: u32 = 0x103;
    pub const LINK_RATE_SET: u32 = 0x115;
    pub const LANE0_1_STATUS: u32 = 0x202;
    pub const LANE_ALIGN_STATUS: u32 = 0x204;
    pub const ADJUST_REQUEST_LANE0_1: u32 = 0x206;
}

pub const CR_DONE: u8 = 1;
pub const EQ_DONE: u8 = 2;
pub const SYMBOL_LOCKED: u8 = 4;
pub const INTERLANE_ALIGN_DONE: u8 = 1;
pub const ENHANCED_FRAME: u8 = 0x80;
pub const SCRAMBLING_DISABLE: u8 = 0x20;
pub const MAX_SWING_REACHED: u8 = 1 << 2;
pub const MAX_PRE_REACHED: u8 = 1 << 5;

/// Training patterns, as the source and DPCD 0x102 number them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern { Off, Tps1, Tps2, Tps3, Tps4 }
impl Pattern {
    pub fn dpcd(self) -> u8 {
        match self {
            Pattern::Off => 0, Pattern::Tps1 => 1 | SCRAMBLING_DISABLE, Pattern::Tps2 => 2 | SCRAMBLING_DISABLE,
            Pattern::Tps3 => 3 | SCRAMBLING_DISABLE, Pattern::Tps4 => 7, // TPS4 is sent scrambled
        }
    }
}

/// What the source side must do during training.
pub trait Source {
    fn set_pattern(&mut self, p: Pattern);
    /// Per-lane (swing, pre-emphasis) levels 0..=3.
    fn set_levels(&mut self, levels: &[(u8, u8); 4]);
    fn dpcd_read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), Error>;
    fn dpcd_write(&mut self, addr: u32, data: &[u8]) -> Result<(), Error>;
    fn delay_us(&mut self, us: u32);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Link rate in kHz of the symbol clock (162000, 270000, 540000, 810000...).
    pub rate_khz: u32,
    pub lanes: u8,
    /// eDP 1.4 rate-table index for non-standard rates (LINK_RATE_SET), else None.
    pub rate_index: Option<u8>,
    pub tps3: bool,
    pub tps4: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { ClockRecovery, Equalisation, Aux(Error) }
impl From<Error> for Failure { fn from(e: Error) -> Self { Failure::Aux(e) } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trained { pub levels: [(u8, u8); 4], pub cr_tries: u32, pub eq_tries: u32, pub pattern: Pattern }

fn lane_bits(status: &[u8; 2], lane: usize) -> u8 { (status[lane / 2] >> ((lane % 2) * 4)) & 0xf }

fn lane_set(levels: &[(u8, u8); 4], lanes: usize) -> [u8; 4] {
    let mut out = [0u8; 4];
    for ln in 0..lanes {
        let (s, p) = levels[ln];
        out[ln] = s | p << 3 | if s == 3 { MAX_SWING_REACHED } else { 0 } | if s + p >= 3 { MAX_PRE_REACHED } else { 0 };
    }
    out
}

/// The levels the sink asks for (DPCD 0x206/0x207), all lanes set to the
/// highest request as DP recommends for a shared PHY, clamped to swing+pre <= 3.
fn requested(adj: &[u8; 2], lanes: usize) -> [(u8, u8); 4] {
    let (mut s, mut p) = (0u8, 0u8);
    for ln in 0..lanes {
        let v = adj[ln / 2] >> ((ln % 2) * 4);
        s = s.max(v & 3);
        p = p.max((v >> 2) & 3);
    }
    let p = p.min(3 - s);
    [(s, p); 4]
}

fn interval_us(src: &mut impl Source, eq: bool) -> u32 {
    let mut b = [0u8];
    let v = if src.dpcd_read(dpcd::TRAINING_AUX_RD_INTERVAL, &mut b).is_ok() { b[0] & 0x7f } else { 0 };
    // DP 1.4: clock recovery always waits 100 us; EQ waits the advertised 4 ms steps (400 us if 0).
    if !eq { 100 } else if v == 0 { 400 } else { v as u32 * 4000 }
}

/// Train the link. On success the sink is still in training-pattern mode
/// with the source on the EQ pattern: the caller switches the source to idle
/// / normal pixels and writes TRAINING_PATTERN_SET = 0 ([`finish`]).
pub fn train(src: &mut impl Source, cfg: &Config) -> Result<Trained, Failure> {
    let lanes = cfg.lanes.clamp(1, 4) as usize;
    match cfg.rate_index {
        Some(i) => { src.dpcd_write(dpcd::LINK_BW_SET, &[0])?; src.dpcd_write(dpcd::LINK_RATE_SET, &[i])?; }
        None => src.dpcd_write(dpcd::LINK_BW_SET, &[(cfg.rate_khz / 27_000) as u8])?,
    }
    src.dpcd_write(dpcd::LANE_COUNT_SET, &[lanes as u8 | ENHANCED_FRAME])?;
    // Clock recovery.
    let mut levels = [(0u8, 0u8); 4];
    src.set_levels(&levels);
    src.set_pattern(Pattern::Tps1);
    let mut msg = [Pattern::Tps1.dpcd(), 0, 0, 0, 0];
    msg[1..5].copy_from_slice(&lane_set(&levels, 4));
    src.dpcd_write(dpcd::TRAINING_PATTERN_SET, &msg[..1 + lanes])?;
    let (mut cr_tries, mut same, mut last_swing) = (0u32, 0u32, 0u8);
    loop {
        let us = interval_us(src, false);
        src.delay_us(us);
        cr_tries += 1;
        let mut st = [0u8; 6];
        src.dpcd_read(dpcd::LANE0_1_STATUS, &mut st)?;
        let status = [st[0], st[1]];
        if (0..lanes).all(|ln| lane_bits(&status, ln) & CR_DONE != 0) { break; }
        let next = requested(&[st[4], st[5]], lanes);
        if levels[0].0 == 3 && next[0].0 == 3 { return Err(Failure::ClockRecovery); }
        if next[0].0 == last_swing { same += 1; } else { same = 0; last_swing = next[0].0; }
        // DP: five tries at one swing, or ten in all, then give up.
        if same >= 5 || cr_tries >= 10 { return Err(Failure::ClockRecovery); }
        levels = next;
        src.set_levels(&levels);
        src.dpcd_write(dpcd::TRAINING_LANE0_SET, &lane_set(&levels, lanes)[..lanes])?;
    }
    // Channel equalisation with the best pattern both ends support.
    let pattern = if cfg.tps4 { Pattern::Tps4 } else if cfg.tps3 { Pattern::Tps3 } else { Pattern::Tps2 };
    src.set_pattern(pattern);
    let mut msg = [pattern.dpcd(), 0, 0, 0, 0];
    msg[1..5].copy_from_slice(&lane_set(&levels, 4));
    src.dpcd_write(dpcd::TRAINING_PATTERN_SET, &msg[..1 + lanes])?;
    for eq_tries in 1..=5 {
        let us = interval_us(src, true);
        src.delay_us(us);
        let mut st = [0u8; 6];
        src.dpcd_read(dpcd::LANE0_1_STATUS, &mut st)?;
        let status = [st[0], st[1]];
        if !(0..lanes).all(|ln| lane_bits(&status, ln) & CR_DONE != 0) { return Err(Failure::ClockRecovery); }
        let eq = (0..lanes).all(|ln| lane_bits(&status, ln) & (EQ_DONE | SYMBOL_LOCKED) == EQ_DONE | SYMBOL_LOCKED);
        if eq && st[2] & INTERLANE_ALIGN_DONE != 0 {
            return Ok(Trained { levels, cr_tries, eq_tries, pattern });
        }
        levels = requested(&[st[4], st[5]], lanes);
        src.set_levels(&levels);
        src.dpcd_write(dpcd::TRAINING_LANE0_SET, &lane_set(&levels, lanes)[..lanes])?;
    }
    Err(Failure::Equalisation)
}

/// End training on the sink side (pattern off, scrambling back on).
pub fn finish(src: &mut impl Source) -> Result<(), Error> { src.dpcd_write(dpcd::TRAINING_PATTERN_SET, &[0]) }
