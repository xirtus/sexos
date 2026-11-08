// SPDX-License-Identifier: GPL-3.0-only
//! A pixel surface for each program row: a BOOT region the program writes and Display
//! reads, so a program can show an image or draw. Holding it grants only that window's
//! pixels; Display takes a size only from the row that owns the surface.
use crate::app_catalog::{App, INCARNATIONS};

/// The largest picture shown, and room behind it for a decoder's own work.
pub const MAX_W: usize = 800;
pub const MAX_H: usize = 600;
pub const PIXELS: usize = MAX_W * MAX_H;
pub const BYTES: u64 = 4 * 1024 * 1024;
const _: () = assert!(PIXELS as u64 * 4 <= BYTES);
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
