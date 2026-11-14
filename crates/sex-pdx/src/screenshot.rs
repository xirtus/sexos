// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Screenshot UI: Print shows a selection over the desktop (drag a region,
//! click a window, Enter for the whole screen, Escape to cancel); Shift+Print
//! takes the whole screen at once. The Shell runs the selection and Display
//! draws it; the first Spindle terminal, which holds the capture grant and
//! the PNG encoder, saves the picture as `/Screenshot-N.png` in Files.
//!
//! Shell -> Display `OP_SCREENSHOT_UI`: arg0 = the selection (`Rect::wire`),
//! arg1 = 1 while the selection UI shows, 0 to hide it.
//! Shell -> Spindle `OP_SCREENSHOT_TAKE`: arg0 = the region (`Rect::wire`;
//! zero is the whole desktop).

pub const OP_SCREENSHOT_UI: u64 = 0x2E2;
pub const OP_SCREENSHOT_TAKE: u64 = 0x2E3;
/// The key code Print Screen arrives as (set-1 SysRq; keypad * keeps 0x37).
pub const KEY_PRINT: u8 = 0x54;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect { pub x: u16, pub y: u16, pub w: u16, pub h: u16 }

impl Rect {
    pub const FULL: Self = Self { x: 0, y: 0, w: 0, h: 0 };
    pub fn is_full(&self) -> bool { self.w == 0 || self.h == 0 }
    pub fn wire(&self) -> u64 { self.x as u64 | (self.y as u64) << 16 | (self.w as u64) << 32 | (self.h as u64) << 48 }
    pub fn decode(word: u64) -> Self {
        Self { x: word as u16, y: (word >> 16) as u16, w: (word >> 32) as u16, h: (word >> 48) as u16 }
    }
    /// From two corners, either order.
    pub fn span(ax: i32, ay: i32, bx: i32, by: i32) -> Self {
        let c = |v: i32| v.clamp(0, u16::MAX as i32) as u16;
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        Self { x: c(x0), y: c(y0), w: c(x1) - c(x0), h: c(y1) - c(y0) }
    }
    /// Within a `width` x `height` picture; None when nothing is left.
    /// The whole-desktop rect stays the whole picture.
    pub fn clamp(&self, width: usize, height: usize) -> Option<(usize, usize, usize, usize)> {
        if width == 0 || height == 0 { return None; }
        if self.is_full() { return Some((0, 0, width, height)); }
        let (x, y) = (self.x as usize, self.y as usize);
        if x >= width || y >= height { return None; }
        Some((x, y, (self.w as usize).min(width - x), (self.h as usize).min(height - y)))
    }
    pub fn contains(&self, px: usize, py: usize) -> bool {
        let (x, y) = (self.x as usize, self.y as usize);
        px >= x && py >= y && px < x + self.w as usize && py < y + self.h as usize
    }
}

/// The index into a `width`-wide picture of pixel `i` of the `region` crop.
pub fn crop_index(region: (usize, usize, usize, usize), width: usize, i: usize) -> usize {
    let (x, y, w, _) = region;
    (y + i / w) * width + x + i % w
}

/// A drag smaller than this either way is a click.
pub const CLICK_SLOP: i32 = 4;

/// What the person chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick { Region(Rect), WindowAt(i32, i32), Full, Cancel }

/// The Print selection: press-drag-release picks a region, a click a window,
/// Enter the whole screen, Escape cancels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selector { open: bool, anchor: Option<(i32, i32)>, at: (i32, i32) }

impl Selector {
    pub const fn new() -> Self { Self { open: false, anchor: None, at: (0, 0) } }
    pub fn is_open(&self) -> bool { self.open }
    pub fn open(&mut self) { *self = Self { open: true, anchor: None, at: (0, 0) }; }
    pub fn press(&mut self, x: i32, y: i32) { if self.open { self.anchor = Some((x, y)); self.at = (x, y); } }
    /// True when the drawn selection changed.
    pub fn motion(&mut self, x: i32, y: i32) -> bool {
        if !self.open || self.anchor.is_none() || self.at == (x, y) { return false; }
        self.at = (x, y);
        true
    }
    pub fn release(&mut self, x: i32, y: i32) -> Option<Pick> {
        let (ax, ay) = self.anchor.take().filter(|_| self.open)?;
        self.open = false;
        Some(if (x - ax).abs() < CLICK_SLOP && (y - ay).abs() < CLICK_SLOP { Pick::WindowAt(x, y) }
             else { Pick::Region(Rect::span(ax, ay, x, y)) })
    }
    pub fn key(&mut self, enter: bool) -> Option<Pick> {
        if !self.open { return None; }
        self.open = false;
        self.anchor = None;
        Some(if enter { Pick::Full } else { Pick::Cancel })
    }
    /// The selection to draw now (empty before a drag starts).
    pub fn selection(&self) -> Rect {
        match self.anchor { Some((ax, ay)) => Rect::span(ax, ay, self.at.0, self.at.1), None => Rect::default() }
    }
}

/// `/Screenshot-N.png` for attempt `n` (1-based), in `out`; returns its length.
pub fn file_name(n: u32, out: &mut [u8; 32]) -> usize {
    let mut digits = [0u8; 10];
    let (mut v, mut d) = (n, 0);
    loop { digits[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; if v == 0 { break; } }
    let mut len = 0;
    for &b in b"/Screenshot-" { out[len] = b; len += 1; }
    for i in (0..d).rev() { out[len] = digits[i]; len += 1; }
    for &b in b".png" { out[len] = b; len += 1; }
    len
}
