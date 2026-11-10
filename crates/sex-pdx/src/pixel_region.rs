// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A pixel surface for each program row: a BOOT region the program writes and Display
//! reads, so a program can show an image or draw. Holding it grants only that window's
//! pixels; Display takes a size only from the row that owns the surface.
use crate::app_catalog::{App, INCARNATIONS};

/// Every surface holds at least an 800 by 600 picture. BOOT sizes the regions to the
/// screen (four bytes per screen pixel, rounded up to 1 MiB), so a full-screen
/// window is all pixels; programs read the real length from their grant.
pub const MAX_W: usize = 800;
pub const MAX_H: usize = 600;
pub const PIXELS: usize = MAX_W * MAX_H;
/// The smallest region BOOT grants.
pub const BYTES: u64 = 4 * 1024 * 1024;
const _: () = assert!(PIXELS as u64 * 4 <= BYTES);
/// The region length BOOT grants for a screen of `width` by `height`.
pub const fn bytes_for_screen(width: u64, height: u64) -> u64 {
    let want = (width * height * 4 + (1 << 20) - 1) & !((1 << 20) - 1);
    if want > BYTES { want } else { BYTES }
}
/// Where a program's pixels start inside its framed window: below the title bar,
/// inside the border. Display draws there and Shell reports the size left.
pub const LEFT: u32 = crate::desktop_style::INSET;
pub const TOP: u32 = crate::desktop_style::chrome::TITLE+crate::desktop_style::chrome::RESIZE;
/// The pixel area of a framed window `width` by `height`.
pub const fn area(width: u32, height: u32) -> (u32, u32) {
    (width.saturating_sub(2 * LEFT), height.saturating_sub(TOP + LEFT))
}
/// Applied device-pixel viewport. Programs receive the resulting dimensions
/// over their held resize route; they need no ambient monitor/scale access.
pub const fn area_scaled(width:u32,height:u32,ui_scale:u32)->(u32,u32){
    let b=crate::desktop_style::chrome::Metrics::new(ui_scale).body(width,height);(b.w,b.h)
}
/// Shell to program, as an input event (`app_key` opcode 0x202): the window's pixel
/// area is now `word >> 32` by `word & 0xFFFF_FFFF`.
pub const RESIZE_CLASS: u64 = 6;
pub const fn resize_wire(width: u32, height: u32) -> u64 { ((width as u64) << 32) | height as u64 }
pub fn resize_decode(word: u64, reserved: u64, class: u64) -> Option<(u32, u32)> {
    if reserved != 0 || class != RESIZE_CLASS { return None; }
    Some(((word >> 32) as u32, word as u32))
}
/// The program's own view, writable.
pub const CLIENT_SLOT: u64 = 97;
/// Display's read-only views, one per program row.
pub const DISPLAY_FIRST_SLOT: u64 = 600;
pub const fn display_slot(index: usize) -> u64 { DISPLAY_FIRST_SLOT + index as u64 }
pub const fn region_id(index: usize) -> u32 { 0x4200 + index as u32 }
/// Program to Display: show `arg1 >> 32` by `arg1 & 0xFFFF_FFFF` pixels of its region
/// (0x00RRGGBB, row-major) in surface `arg0`. A size of zero goes back to text.
pub const OP: u64 = 0x2D2;
/// The program row's position among program rows.
pub fn index_of(target: u32) -> Option<usize> {
    INCARNATIONS.iter().filter(|r| r.app == App::Program).position(|r| r.target == target)
}
