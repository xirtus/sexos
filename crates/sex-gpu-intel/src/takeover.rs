// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Taking the panel over from firmware without a modeset (slice L3).
//!
//! GOP left a pipe running at the panel's mode, scanning a linear framebuffer
//! in stolen memory. SexOS keeps the pipe, transcoder, link and timings exactly
//! as they are and only repoints the primary plane at its own two buffers,
//! flipping between them at vblank. Nothing here can blank the panel: every
//! step that fails puts the firmware plane back.

use crate::plane::{self, Surface};
use crate::readout::{read_pipe, PipeReadout};
use crate::regs::*;
use crate::{Error, Mmio};

/// The firmware plane, saved so a failed or abandoned takeover restores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved { pub pipe: u32, pub ctl: u32, pub stride: u32, pub pos: u32, pub size: u32, pub offset: u32, pub surf: u32 }

pub fn save(m: &mut impl Mmio, p: u32) -> Saved {
    Saved {
        pipe: p,
        ctl: m.read(plane(p, 1, PLANE_CTL)),
        stride: m.read(plane(p, 1, PLANE_STRIDE)),
        pos: m.read(plane(p, 1, PLANE_POS)),
        size: m.read(plane(p, 1, PLANE_SIZE)),
        offset: m.read(plane(p, 1, PLANE_OFFSET)),
        surf: m.read(plane(p, 1, PLANE_SURF)),
    }
}

/// Put the firmware plane back (SURF last, so it latches as one update).
pub fn restore(m: &mut impl Mmio, s: &Saved) {
    let p = s.pipe;
    m.write(plane(p, 1, PLANE_CTL), s.ctl);
    m.write(plane(p, 1, PLANE_STRIDE), s.stride);
    m.write(plane(p, 1, PLANE_POS), s.pos);
    m.write(plane(p, 1, PLANE_SIZE), s.size);
    m.write(plane(p, 1, PLANE_OFFSET), s.offset);
    m.write(plane(p, 1, PLANE_SURF), s.surf);
}

/// The pipe that shows the boot framebuffer: running, with a linear XRGB
/// primary plane exactly `width` x `height`. Pipe A (the panel) wins a tie.
pub fn boot_pipe(m: &mut impl Mmio, width: u32, height: u32) -> Option<PipeReadout> {
    (0..PIPES).map(|p| read_pipe(m, p)).find(|r| {
        r.enabled && r.plane.enabled && r.plane.linear_xrgb() && r.plane.width == width && r.plane.height == height
    })
}

/// Why a takeover was refused: every case leaves the firmware plane showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal { NoPipe, Psr2, Surface, NoLatch, Device(Error) }
impl Refusal {
    pub fn name(&self) -> &'static str {
        match self { Refusal::NoPipe => "no_matching_pipe", Refusal::Psr2 => "psr2_selective_update", Refusal::Surface => "surface", Refusal::NoLatch => "flip_never_latched", Refusal::Device(_) => "device" }
    }
}

/// Flip-chain over two buffers on one pipe's primary plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    pub pipe: u32,
    pub buffers: [u32; 2],
    pub stride_bytes: u32,
    pub width: u32,
    pub height: u32,
    /// The buffer the plane scans (or will scan once the armed flip latches).
    pub front: usize,
    /// A flip armed but not yet confirmed on SURFLIVE.
    pub pending: bool,
    pub flips: u64,
    /// Vblanks we had to wait for before writing the back buffer.
    pub waits: u64,
    pub saved: Saved,
}

/// Wait until the plane's live surface is `ggtt`. The frame counter is read
/// between checks, so the wait spans vblanks even on a device that only
/// updates SURFLIVE when it latches. `budget` bounds the reads.
pub fn wait_latched(m: &mut impl Mmio, p: u32, ggtt: u32, budget: u32) -> Result<(), Error> {
    for _ in 0..budget.max(1) {
        if plane::flip_done(m, p, ggtt) { return Ok(()); }
        let _ = plane::frame_count(m, p);
    }
    Err(Error::Timeout(plane(p, 1, PLANE_SURFLIVE)))
}

impl Chain {
    /// Repoint the boot pipe's primary plane at buffer 0. The caller has
    /// already copied the boot picture into it, so the switch is invisible.
    /// The same mode, same position: only stride, format bits and surface change.
    pub fn take(m: &mut impl Mmio, width: u32, height: u32, buffers: [u32; 2], stride_bytes: u32, budget: u32) -> Result<Chain, Refusal> {
        let r = boot_pipe(m, width, height).ok_or(Refusal::NoPipe)?;
        // PSR2 on Gen12 updates the panel only where software says it changed
        // (selective fetch); flipping without that could freeze the picture.
        // PSR1 exits on a plane flip by itself, so it is fine.
        if r.psr2 { return Err(Refusal::Psr2); }
        let s = Surface { ggtt: buffers[0], stride_bytes, width, height };
        if !s.valid() || buffers[1] % SURF_ALIGN != 0 { return Err(Refusal::Surface); }
        let saved = save(m, r.pipe);
        plane::set_primary(m, r.pipe, &s, r.plane.x, r.plane.y).map_err(Refusal::Device)?;
        if wait_latched(m, r.pipe, buffers[0], budget).is_err() {
            restore(m, &saved);
            return Err(Refusal::NoLatch);
        }
        Ok(Chain { pipe: r.pipe, buffers, stride_bytes, width, height, front: 0, pending: false, flips: 0, waits: 0, saved })
    }

    /// The buffer to draw the next frame into. Waits (bounded) until the last
    /// flip has latched, so the engine is no longer reading it: tear-free, and
    /// at most one flip per refresh, which paces the desktop to the panel.
    pub fn back(&mut self, m: &mut impl Mmio, budget: u32) -> Result<usize, Error> {
        if self.pending {
            if !plane::flip_done(m, self.pipe, self.buffers[self.front]) { self.waits += 1; }
            wait_latched(m, self.pipe, self.buffers[self.front], budget)?;
            self.pending = false;
        }
        Ok(1 - self.front)
    }

    /// Show the back buffer from the next vblank on.
    pub fn flip(&mut self, m: &mut impl Mmio) -> Result<(), Error> {
        let back = 1 - self.front;
        plane::flip(m, self.pipe, self.buffers[back])?;
        self.front = back;
        self.pending = true;
        self.flips += 1;
        Ok(())
    }

    /// Hand the panel back to the firmware framebuffer (fallback path).
    pub fn release(&self, m: &mut impl Mmio) {
        plane::cursor_off(m, self.pipe);
        restore(m, &self.saved);
    }
}

/// The hardware cursor on the chain's pipe: one ARGB image, moved by
/// register writes only. Coordinates are in panel pixels, top-left hotspot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor { pub pipe: u32, pub size: u32, pub ggtt: u32, pub shown: bool, pub x: i32, pub y: i32 }
impl Cursor {
    pub fn new(pipe: u32, size: u32, ggtt: u32) -> Option<Cursor> {
        plane::cursor_mode(size)?;
        (ggtt % SURF_ALIGN == 0).then_some(Cursor { pipe, size, ggtt, shown: false, x: 0, y: 0 })
    }
    /// Show at (x, y), or just move when already shown. No-op when unchanged.
    pub fn show(&mut self, m: &mut impl Mmio, x: i32, y: i32) -> Result<(), Error> {
        if self.shown && self.x == x && self.y == y { return Ok(()); }
        if self.shown { plane::cursor_move(m, self.pipe, x, y); }
        else { plane::cursor_set(m, self.pipe, self.size, self.ggtt, x, y)?; self.shown = true; }
        self.x = x; self.y = y;
        Ok(())
    }
    pub fn hide(&mut self, m: &mut impl Mmio) {
        if self.shown { plane::cursor_off(m, self.pipe); self.shown = false; }
    }
}
