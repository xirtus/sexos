// SPDX-License-Identifier: GPL-3.0-only
//! RFB, the remote framebuffer protocol VNC speaks (RFC 6143, version 3.8, with 3.3 and
//! 3.7 servers understood). No I/O here: [`client::Client`] is a state machine fed the
//! bytes that arrive and drained of the bytes to send, so Lurk drives it over a SexOS
//! TCP flow and the host tests drive it over byte strings. [`server`] has the server's
//! message encoders and a parser for what clients send, for Lurk's server half.
//!
//! Pixels are always 32 bits, `0x00RRGGBB` little-endian ([`PixelFormat::RGB32`]): the
//! client asks for it right after ServerInit, which is also the layout of a SexOS
//! window's pixels.
#![no_std]

extern crate alloc;

pub mod client;
pub mod des;
pub mod keysym;
pub mod server;
mod zrle;

use alloc::vec::Vec;

/// Encoding numbers (RFC 6143 section 7.7 and the IANA registry).
pub mod encoding {
    pub const RAW: i32 = 0;
    pub const COPY_RECT: i32 = 1;
    pub const RRE: i32 = 2;
    pub const HEXTILE: i32 = 5;
    pub const ZRLE: i32 = 16;
    pub const CURSOR: i32 = -239;
    pub const DESKTOP_SIZE: i32 = -223;
}

/// Security types.
pub mod security {
    pub const NONE: u8 = 1;
    pub const VNC_AUTH: u8 = 2;
}

/// The largest framebuffer accepted, so a hostile server can't ask for all memory.
pub const MAX_SIDE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// 32 bits, depth 24, little-endian, red in bits 16..24: `0x00RRGGBB`.
    pub const RGB32: PixelFormat = PixelFormat { bits_per_pixel: 32, depth: 24, big_endian: false, true_colour: true, red_max: 255, green_max: 255, blue_max: 255, red_shift: 16, green_shift: 8, blue_shift: 0 };

    pub fn to_bytes(&self) -> [u8; 16] {
        let [rh, rl] = self.red_max.to_be_bytes();
        let [gh, gl] = self.green_max.to_be_bytes();
        let [bh, bl] = self.blue_max.to_be_bytes();
        [self.bits_per_pixel, self.depth, self.big_endian as u8, self.true_colour as u8, rh, rl, gh, gl, bh, bl, self.red_shift, self.green_shift, self.blue_shift, 0, 0, 0]
    }
    pub fn from_bytes(b: &[u8; 16]) -> PixelFormat {
        PixelFormat {
            bits_per_pixel: b[0], depth: b[1], big_endian: b[2] != 0, true_colour: b[3] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]), green_max: u16::from_be_bytes([b[6], b[7]]), blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10], green_shift: b[11], blue_shift: b[12],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The other side broke the protocol; the text says where.
    Protocol(&'static str),
    /// The server refused the connection and said why.
    Refused(Vec<u8>),
    /// The server wants a password and none was given.
    NeedPassword,
    /// The password was wrong (the server's reason, if it gave one).
    AuthFailed(Vec<u8>),
    /// No security type both sides know.
    NoSecurity,
    /// The server used an encoding the client did not ask for.
    Encoding(i32),
    /// The framebuffer would be larger than [`MAX_SIDE`] on a side.
    TooLarge,
}

/// A remote screen: `width` by `height` pixels, `0x00RRGGBB`, row-major.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Framebuffer {
        Framebuffer { width, height, pixels: alloc::vec![0; width * height] }
    }
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u32) {
        for row in y..y + h { self.pixels[row * self.width + x..row * self.width + x + w].fill(colour); }
    }
    /// Copies the `w` by `h` block at `(sx, sy)` to `(x, y)`; the two may overlap.
    pub fn copy(&mut self, sx: usize, sy: usize, x: usize, y: usize, w: usize, h: usize) {
        let rows: Vec<usize> = if sy < y { (0..h).rev().collect() } else { (0..h).collect() };
        for r in rows {
            let from = (sy + r) * self.width + sx;
            self.pixels.copy_within(from..from + w, (y + r) * self.width + x);
        }
    }
    pub fn contains(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        x + w <= self.width && y + h <= self.height
    }
}

/// A rectangle of the screen, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect { pub x: u16, pub y: u16, pub w: u16, pub h: u16 }

impl Rect {
    /// The smallest rectangle holding both.
    pub fn union(self, o: Rect) -> Rect {
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        let (x1, y1) = ((self.x + self.w).max(o.x + o.w), (self.y + self.h).max(o.y + o.h));
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// A forward-only reader over bytes that may not all have arrived: `None` means wait.
pub(crate) struct Reader<'a> { pub buf: &'a [u8], pub at: usize }
impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self { Reader { buf, at: 0 } }
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.buf.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> { self.bytes(1).map(|b| b[0]) }
    pub fn u16(&mut self) -> Option<u16> { self.bytes(2).map(|b| u16::from_be_bytes([b[0], b[1]])) }
    pub fn u32(&mut self) -> Option<u32> { self.bytes(4).map(|b| u32::from_be_bytes(b.try_into().unwrap())) }
    /// A pixel in [`PixelFormat::RGB32`].
    pub fn pixel(&mut self) -> Option<u32> { self.bytes(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) & 0x00FF_FFFF) }
}

#[cfg(test)]
mod tests;
