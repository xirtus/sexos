// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Wasm layout plugins (slice S4 follow-up): the wire between the Shell and a
//! plugin's own run of the `wasm` program (`wasm layout @FILE`, or
//! `wasm layout spiral` for the built-in one).
//!
//! The plugin never runs inside the Shell. When a workspace that uses it
//! retiles, the Shell sends the run [`OP_LAYOUT_ASK`] as a plain call it does
//! not wait on, and keeps tiling with the workspace's previous layout. The run
//! calls the guest's `layout(n, w, h) -> ptr` export with a fuel limit, and
//! sends back each rectangle as its own call, [`OP_LAYOUT_RECT`], or why there
//! are none, [`OP_LAYOUT_FAIL`]. The Shell checks the whole answer before it
//! uses any of it; an answer for an older generation is dropped.
//!
//! Rectangles are relative to the work area's top-left corner: `x`, `y` as
//! `i32`, `w`, `h` as `u32`, in that order, sixteen bytes each in the guest's
//! memory and on the wire.
//!
//! Self-contained (no other module) so host tests compile it alone.

/// Shell -> plugin run: `[generation, count, w << 32 | h]`.
pub const OP_LAYOUT_ASK: u64 = 0x310;
/// Plugin run -> Shell: one rectangle, `[generation << 32 | index << 16 | count,
/// x << 32 | y, w << 32 | h]`.
pub const OP_LAYOUT_RECT: u64 = 0x311;
/// Plugin run -> Shell: `[generation, reason]`, see [`Fail`].
pub const OP_LAYOUT_FAIL: u64 = 0x312;

/// The first argument of a plugin's run: `wasm layout @/path/x.wasm`.
pub const MODE: &[u8] = b"layout";
/// The most windows one answer places (the Shell's frames).
pub const MAX_RECTS: u32 = 24;
/// Instructions' worth of fuel a single `layout` call may use.
pub const FUEL_PER_CALL: u64 = 2_000_000;
/// The largest linear memory a plugin may have.
pub const MEMORY_BYTES: usize = 1 << 20;
/// The largest plugin module.
pub const MODULE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ask { pub generation: u32, pub count: u32, pub w: u32, pub h: u32 }

impl Ask {
    pub fn wire(self) -> [u64; 3] { [self.generation as u64, self.count as u64, (self.w as u64) << 32 | self.h as u64] }
    pub fn decode(w: [u64; 3]) -> Option<Ask> {
        if w[0] >> 32 != 0 || w[1] == 0 || w[1] > MAX_RECTS as u64 { return None; }
        let (width, height) = ((w[2] >> 32) as u32, w[2] as u32);
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 { return None; }
        Some(Ask { generation: w[0] as u32, count: w[1] as u32, w: width, h: height })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectMsg { pub generation: u32, pub index: u16, pub count: u16, pub rect: Rect }

impl RectMsg {
    pub fn wire(self) -> [u64; 3] {
        let r = self.rect;
        [(self.generation as u64) << 32 | (self.index as u64) << 16 | self.count as u64,
         (r.x as u32 as u64) << 32 | r.y as u32 as u64,
         (r.w as u64) << 32 | r.h as u64]
    }
    pub fn decode(w: [u64; 3]) -> Option<RectMsg> {
        let (index, count) = ((w[0] >> 16) as u16, w[0] as u16);
        if count == 0 || count as u32 > MAX_RECTS || index >= count { return None; }
        let rect = Rect { x: (w[1] >> 32) as u32 as i32, y: w[1] as u32 as i32, w: (w[2] >> 32) as u32, h: w[2] as u32 };
        Some(RectMsg { generation: (w[0] >> 32) as u32, index, count, rect })
    }
}

/// Why a plugin run sent no rectangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The module could not be read (missing file, not granted).
    Read = 1,
    /// Not a valid Wasm module, or it lacks `memory`/`layout`, or imports anything.
    Module = 2,
    /// The call ran out of fuel (an endless loop, or simply too slow).
    Fuel = 3,
    /// The guest trapped (out-of-bounds access, unreachable, division by zero...).
    Trap = 4,
    /// The returned pointer does not hold `count` rectangles inside its memory, or
    /// the guest returned a negative value.
    Result = 5,
}

impl Fail {
    pub const ALL: [Fail; 5] = [Fail::Read, Fail::Module, Fail::Fuel, Fail::Trap, Fail::Result];
    pub fn wire(self, generation: u32) -> [u64; 3] { [generation as u64, self as u64, 0] }
    pub fn decode(w: [u64; 3]) -> Option<(u32, Fail)> {
        if w[0] >> 32 != 0 || w[2] != 0 { return None; }
        Some((w[0] as u32, *Fail::ALL.iter().find(|f| **f as u64 == w[1])?))
    }
    pub fn name(self) -> &'static str {
        match self { Fail::Read => "read", Fail::Module => "module", Fail::Fuel => "fuel", Fail::Trap => "trap", Fail::Result => "result" }
    }
}
