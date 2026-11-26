// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A pixel surface for each program row: a BOOT region the program writes and Display
//! reads, so a program can show an image or draw. Holding it grants only that window's
//! pixels; Display takes a size only from the row that owns the surface.
//!
//! The region holds two pages. A program paints the page Display is not showing and
//! names it in [`OP`]; Display reads only the page named last, and answers with
//! [`shown`] once it has switched, so the program never paints a page that is on
//! screen and Display never shows a frame half drawn. With room to spare BOOT makes
//! each page a whole screen; on a small machine the region is one screen and its
//! pages are halves, and a window too big for a half shows the [`WHOLE`] region,
//! painted in place.
use crate::app_catalog::{App, INCARNATIONS};

/// Every page holds at least an 800 by 600 picture. BOOT sizes each page to the
/// screen (four bytes per screen pixel, rounded up to 1 MiB), so a full-screen
/// window is all pixels; programs read the real length from their grant.
pub const MAX_W: usize = 800;
pub const MAX_H: usize = 600;
pub const PIXELS: usize = MAX_W * MAX_H;
/// The smallest page BOOT grants (a region is [`PAGES`] of them).
pub const BYTES: u64 = 4 * 1024 * 1024;
/// Pages in each region: the one on screen and the one being painted.
pub const PAGES: u64 = 2;
const _: () = assert!(PIXELS as u64 * 4 <= BYTES);
/// The page length for a screen of `width` by `height`.
pub const fn bytes_for_screen(width: u64, height: u64) -> u64 {
    let want = (width * height * 4 + (1 << 20) - 1) & !((1 << 20) - 1);
    if want > BYTES { want } else { BYTES }
}
/// The region length BOOT grants for a screen of `width` by `height`: [`PAGES`] whole
/// screens when memory is `roomy`, else one screen split into two half pages.
pub const fn region_for_screen(width: u64, height: u64, roomy: bool) -> u64 {
    if roomy { PAGES * bytes_for_screen(width, height) } else { bytes_for_screen(width, height) }
}
/// Machines with at least this much free memory at boot get whole-screen pages.
pub const ROOMY_BYTES: u64 = 1536 << 20;
/// [`OP`]'s page for the whole region from its start, painted in place: for a frame
/// bigger than a page.
pub const WHOLE: u64 = 2;
/// One page's length in a region `region` bytes long; page `p` starts at `p` times it.
pub const fn page_bytes(region: u64) -> u64 { region / PAGES }
/// Where page `page` (0, 1 or [`WHOLE`]) starts in a region `region` bytes long, and
/// how many bytes it holds.
pub const fn page_span(region: u64, page: u64) -> (u64, u64) {
    if page == WHOLE { (0, region) } else { ((page & 1) * page_bytes(region), page_bytes(region)) }
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
/// Program to Display: show `arg1 >> 32` by `arg1 & 0xFFFF_FFFF` pixels of page `arg2`
/// (0, 1 or [`WHOLE`]) of its region (0x00RRGGBB, row-major) in surface `arg0`. A size of zero
/// goes back to text. Display answers [`shown`]`(page)` once it reads that page; from
/// then on the other page is the program's to paint.
pub const OP: u64 = 0x2D2;
/// [`OP`] with this as arg1 changes nothing: Display answers [`shown`] with the page
/// it reads for the caller, for a program whose first answer went astray.
pub const ASK: u64 = u64::MAX;
const SHOWN_TAG: u64 = 0x5049_5853_0000_0000;
/// Display's answer to [`OP`]: it now reads `page` and no longer the other.
pub const fn shown(page: u64) -> u64 { SHOWN_TAG | (page & 3) }
/// The page an answer from Display names, when it is [`shown`].
pub const fn shown_page(value: u64) -> Option<u64> {
    if value & !3 == SHOWN_TAG && value & 3 <= WHOLE { Some(value & 3) } else { None }
}
/// The program row's position among program rows.
pub fn index_of(target: u32) -> Option<usize> {
    INCARNATIONS.iter().filter(|r| r.app == App::Program).position(|r| r.target == target)
}
