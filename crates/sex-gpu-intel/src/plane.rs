// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Plane programming: page flips on the primary plane, the hardware cursor,
//! and vblank waits on the pipe frame counter. Every plane latches its
//! registers at the next vblank after its surface register is written, so a
//! flip is tear-free by construction.

use crate::regs::*;
use crate::{wait_for, Error, Mmio};

/// A framebuffer the display engine can scan: GGTT address and layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface { pub ggtt: u32, pub stride_bytes: u32, pub width: u32, pub height: u32 }
impl Surface {
    pub fn valid(&self) -> bool {
        self.ggtt % SURF_ALIGN == 0 && self.stride_bytes % STRIDE_UNIT == 0 && self.stride_bytes >= self.width * 4
            && self.width > 0 && self.height > 0 && self.width <= 5120 && self.height <= 4096
            && self.stride_bytes / STRIDE_UNIT <= 0x7ff
    }
}

/// Program the primary plane to show `s` at (x, y), linear XRGB8888, keeping
/// firmware's other control bits (gamma, CSC, pipe bottom colour). SURF is
/// written last because it arms the update.
pub fn set_primary(m: &mut impl Mmio, p: u32, s: &Surface, x: u32, y: u32) -> Result<(), Error> {
    if !s.valid() { return Err(Error::BadArgument); }
    let ctl = m.read(plane(p, 1, PLANE_CTL));
    m.write(plane(p, 1, PLANE_CTL), (ctl & !(PLANE_FORMAT_MASK | PLANE_TILED_MASK)) | PLANE_ENABLE | PLANE_FORMAT_XRGB8888 | PLANE_TILED_LINEAR);
    m.write(plane(p, 1, PLANE_STRIDE), s.stride_bytes / STRIDE_UNIT);
    m.write(plane(p, 1, PLANE_POS), y << 16 | x);
    m.write(plane(p, 1, PLANE_SIZE), (s.height - 1) << 16 | (s.width - 1));
    m.write(plane(p, 1, PLANE_OFFSET), 0);
    m.write(plane(p, 1, PLANE_SURF), s.ggtt);
    Ok(())
}

/// Page flip: same geometry, new surface. Takes effect at the next vblank.
pub fn flip(m: &mut impl Mmio, p: u32, ggtt: u32) -> Result<(), Error> {
    if ggtt % SURF_ALIGN != 0 { return Err(Error::BadArgument); }
    m.write(plane(p, 1, PLANE_SURF), ggtt);
    Ok(())
}

/// The flip reached the screen: the live surface is the one we armed.
pub fn flip_done(m: &mut impl Mmio, p: u32, ggtt: u32) -> bool {
    m.read(plane(p, 1, PLANE_SURFLIVE)) & !(SURF_ALIGN - 1) == ggtt
}

pub fn frame_count(m: &mut impl Mmio, p: u32) -> u32 { m.read(pipe(p, FRMCOUNT)) }

/// Wait for the pipe's frame counter to advance (one vblank).
pub fn wait_vblank(m: &mut impl Mmio, p: u32, budget: u32) -> Result<u32, Error> {
    let start = frame_count(m, p);
    for _ in 0..budget.max(1) {
        let now = frame_count(m, p);
        if now != start { return Ok(now); }
    }
    Err(Error::Timeout(pipe(p, FRMCOUNT)))
}

pub fn cursor_mode(size: u32) -> Option<u32> {
    match size { 64 => Some(CUR_MODE_64_ARGB), 128 => Some(CUR_MODE_128_ARGB), 256 => Some(CUR_MODE_256_ARGB), _ => None }
}

/// CUR_POS encoding: magnitude with a sign bit per axis, so a cursor can hang
/// off the top or left edge.
pub fn cursor_pos(x: i32, y: i32) -> u32 {
    let axis = |v: i32| -> u32 { let a = v.unsigned_abs() & 0x7fff; if v < 0 { a | CUR_POS_SIGN } else { a } };
    axis(y) << 16 | axis(x)
}

/// Show an ARGB cursor image (64, 128 or 256 square) from a GGTT address.
pub fn cursor_set(m: &mut impl Mmio, p: u32, size: u32, ggtt: u32, x: i32, y: i32) -> Result<(), Error> {
    let mode = cursor_mode(size).ok_or(Error::BadArgument)?;
    if ggtt % SURF_ALIGN != 0 { return Err(Error::BadArgument); }
    m.modify(cursor(p, CUR_CTL), CUR_MODE_MASK, mode);
    m.write(cursor(p, CUR_POS), cursor_pos(x, y));
    m.write(cursor(p, CUR_BASE), ggtt);
    Ok(())
}

/// Move the cursor. Position latches with the base register, so it is
/// rewritten unchanged to arm the move; no pixels are copied anywhere.
pub fn cursor_move(m: &mut impl Mmio, p: u32, x: i32, y: i32) {
    m.write(cursor(p, CUR_POS), cursor_pos(x, y));
    let base = m.read(cursor(p, CUR_BASE));
    m.write(cursor(p, CUR_BASE), base);
}

pub fn cursor_off(m: &mut impl Mmio, p: u32) {
    m.modify(cursor(p, CUR_CTL), CUR_MODE_MASK, CUR_MODE_OFF);
    let base = m.read(cursor(p, CUR_BASE));
    m.write(cursor(p, CUR_BASE), base);
}

/// The scanline the pipe is drawing, for "are we in vblank" decisions.
pub fn scanline(m: &mut impl Mmio, p: u32) -> u32 { m.read(pipe(p, PIPEDSL)) & DSL_MASK }

/// Wait until the transcoder reports it is running (after enabling a pipe).
pub fn wait_pipe_on(m: &mut impl Mmio, p: u32, budget: u32) -> Result<(), Error> {
    wait_for(m, pipe(p, CONF), CONF_STATE, CONF_STATE, budget).map(|_| ())
}
