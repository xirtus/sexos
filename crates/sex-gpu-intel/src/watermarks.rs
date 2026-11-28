// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Plane watermarks for a faster pixel rate on the same raster.
//!
//! Each plane fetches from memory into its share of the display buffer
//! (DBUF, 512-byte blocks). A watermark level says how many blocks (and
//! lines) must be buffered to ride out one memory-latency tier; deeper levels
//! let the memory sleep longer. Firmware sized them for its pixel rate. The
//! PRM's two methods (Vol 12 "Watermarks": bytes over the latency, or whole
//! lines over the latency) both grow linearly with the pixel rate, so
//! firmware's values scaled up by the rate ratio, rounded up, plus one line
//! of blocks for a method change, bound what the faster mode needs. That
//! avoids re-deriving memory latency and SAGV block time from pcode.
//!
//! A level that no longer fits the plane's DBUF share is turned off (the
//! memory just sleeps less deeply); level 0 must fit or the change is refused.
//! Levels are never lowered by [`scale`] below firmware's values.

use crate::Mmio;

pub const LEVELS: usize = 8;
pub const WM_ENABLE: u32 = 1 << 31;
pub const WM_IGNORE_LINES: u32 = 1 << 30;
pub const WM_LINES_SHIFT: u32 = 14;
pub const WM_LINES_MASK: u32 = 0x1fff << 14;
pub const WM_BLOCKS_MASK: u32 = 0x1fff;
pub const BUF_END_SHIFT: u32 = 16;
pub const BUF_MASK: u32 = 0x1fff;
pub const BLOCK_BYTES: u32 = 512;

/// Universal plane `n` (1-based) of pipe `p`.
pub const fn plane_wm(p: u32, n: u32, level: u32) -> u32 { 0x70240 + p * 0x1000 + (n - 1) * 0x100 + level * 4 }
pub const fn plane_wm_trans(p: u32, n: u32) -> u32 { 0x70268 + p * 0x1000 + (n - 1) * 0x100 }
pub const fn plane_buf_cfg(p: u32, n: u32) -> u32 { 0x7027c + p * 0x1000 + (n - 1) * 0x100 }
pub const fn cur_wm(p: u32, level: u32) -> u32 { 0x70140 + p * 0x1000 + level * 4 }
pub const fn cur_wm_trans(p: u32) -> u32 { 0x70168 + p * 0x1000 }
pub const fn cur_buf_cfg(p: u32) -> u32 { 0x7017c + p * 0x1000 }

/// Which plane: a universal plane number, or the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which { Plane(u32), Cursor }

/// One plane's watermark registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Wm { pub levels: [u32; LEVELS], pub trans: u32, pub buf: u32 }

impl Wm {
    /// Blocks of DBUF the plane owns (end is inclusive in hardware).
    pub fn ddb_blocks(&self) -> u32 {
        let (start, end) = (self.buf & BUF_MASK, (self.buf >> BUF_END_SHIFT) & BUF_MASK);
        if end == 0 && start == 0 { 0 } else { end.saturating_sub(start) + 1 }
    }
    pub fn enabled_levels(&self) -> usize { self.levels.iter().take_while(|l| *l & WM_ENABLE != 0).count() }
}

fn regs(p: u32, w: Which) -> ([u32; LEVELS], u32, u32) {
    let mut l = [0; LEVELS];
    for (i, r) in l.iter_mut().enumerate() {
        *r = match w { Which::Plane(n) => plane_wm(p, n, i as u32), Which::Cursor => cur_wm(p, i as u32) };
    }
    match w {
        Which::Plane(n) => (l, plane_wm_trans(p, n), plane_buf_cfg(p, n)),
        Which::Cursor => (l, cur_wm_trans(p), cur_buf_cfg(p)),
    }
}

pub fn read(m: &mut impl Mmio, p: u32, w: Which) -> Wm {
    let (l, t, b) = regs(p, w);
    let mut out = Wm { trans: m.read(t), buf: m.read(b), ..Wm::default() };
    for i in 0..LEVELS { out.levels[i] = m.read(l[i]); }
    out
}

/// Write the levels (the DBUF share stays). They latch when the plane's
/// surface (cursor: base) register is next written.
pub fn write(m: &mut impl Mmio, p: u32, w: Which, wm: &Wm) {
    let (l, t, _) = regs(p, w);
    for i in 0..LEVELS { m.write(l[i], wm.levels[i]); }
    m.write(t, wm.trans);
}

/// One level scaled by `num/den` (> 1): blocks and lines rounded up, then a
/// line of blocks more. Disabled when it no longer fits `ddb` blocks with the
/// 10 % the PRM keeps spare (min DDB = blocks + blocks / 10).
fn scale_one(v: u32, num: u64, den: u64, line_blocks: u32, ddb: u32) -> u32 {
    if v & WM_ENABLE == 0 { return v; }
    let up = |x: u32| -> u64 { (x as u64 * num).div_ceil(den) };
    let blocks = up(v & WM_BLOCKS_MASK) + line_blocks as u64;
    let lines = up((v & WM_LINES_MASK) >> WM_LINES_SHIFT) + 1;
    if blocks > WM_BLOCKS_MASK as u64 || lines > 0x1fff || blocks + blocks.div_ceil(10) > ddb as u64 { return 0; }
    (v & !(WM_BLOCKS_MASK | WM_LINES_MASK)) | (lines as u32) << WM_LINES_SHIFT | blocks as u32
}

/// Watermarks for a pixel rate `to_khz` from firmware's at `from_khz`, for a
/// plane `width` pixels wide at 4 bytes per pixel. None when level 0 cannot
/// fit the plane's DBUF share. Lower rates keep firmware's values: they
/// already cover more than the slower fetch needs.
pub fn scale(wm: &Wm, from_khz: u32, to_khz: u32, width: u32) -> Option<Wm> {
    if from_khz == 0 { return None; }
    if to_khz <= from_khz { return Some(*wm); }
    let ddb = wm.ddb_blocks();
    let line_blocks = (width * 4).div_ceil(BLOCK_BYTES);
    let mut out = *wm;
    let mut off = false;
    for i in 0..LEVELS {
        // Levels stay monotonic: once one is off, every deeper one is too.
        out.levels[i] = if off { wm.levels[i] & !WM_ENABLE } else { scale_one(wm.levels[i], to_khz as u64, from_khz as u64, line_blocks, ddb) };
        if wm.levels[i] & WM_ENABLE != 0 && out.levels[i] & WM_ENABLE == 0 { off = true; out.levels[i] = wm.levels[i] & !WM_ENABLE; }
    }
    if wm.levels[0] & WM_ENABLE != 0 && out.levels[0] & WM_ENABLE == 0 { return None; }
    out.trans = scale_one(wm.trans, to_khz as u64, from_khz as u64, line_blocks, ddb);
    if wm.trans & WM_ENABLE != 0 && out.trans & WM_ENABLE == 0 { out.trans = wm.trans & !WM_ENABLE; }
    Some(out)
}

/// Watermarks computed from memory latency (PRM Vol 12 "Display
/// watermark programming") for a linear, unrotated plane: method 1 (bytes
/// fetched over the latency) and method 2 (whole lines over the latency),
/// method 2 once the latency spans a line. Level 0 covers SAGV too (the
/// Gen12 rule when SAGV may run: latency + block time). Levels that do not
/// fit `ddb_blocks` are off; None when level 0 does not fit.
pub fn compute(latency_us: &[u16; LEVELS], sagv_us: u32, pixel_khz: u32, htotal: u32, width: u32, cpp: u32, ddb_blocks: u32) -> Option<[u32; LEVELS]> {
    if pixel_khz == 0 || htotal == 0 { return None; }
    let bytes_per_line = width * cpp;
    // Linear: blocks per line rounded up, plus one (Gen10+).
    let bpl = bytes_per_line.div_ceil(BLOCK_BYTES) + 1;
    let linetime_us = (htotal as u64 * 1000).div_ceil(pixel_khz as u64) as u32;
    let mut out = [0u32; LEVELS];
    let (mut prev_blocks, mut prev_lines) = (0u32, 0u32);
    for l in 0..LEVELS {
        let mut lat = latency_us[l] as u32;
        if lat == 0 { break; }
        if l == 0 { lat += sagv_us; }
        // method1 = lat * rate * cpp / (1000 * 512) + 1 (fixed point rounded up)
        let m1 = (lat as u64 * pixel_khz as u64 * cpp as u64).div_ceil(1000 * BLOCK_BYTES as u64) + 1;
        // method2 = ceil(lat * rate / (htotal * 1000)) * blocks_per_line
        let lines_over = (lat as u64 * pixel_khz as u64).div_ceil(htotal as u64 * 1000);
        let m2 = lines_over * bpl as u64;
        let tiny = cpp * htotal / BLOCK_BYTES < 1 && bytes_per_line / BLOCK_BYTES < 1;
        let sel = if tiny || lat >= linetime_us { m2 } else { m1 };
        let mut blocks = (sel + 1).max(bpl as u64) as u32;
        let mut lines = (sel.div_ceil(bpl as u64)) as u32;
        blocks = blocks.max(prev_blocks);
        lines = lines.max(prev_lines);
        let min_ddb = blocks + blocks.div_ceil(10);
        if min_ddb > ddb_blocks || blocks > WM_BLOCKS_MASK || lines > 31 {
            if l == 0 { return None; }
            break;
        }
        out[l] = WM_ENABLE | lines << WM_LINES_SHIFT | blocks;
        prev_blocks = blocks; prev_lines = lines;
    }
    Some(out)
}
