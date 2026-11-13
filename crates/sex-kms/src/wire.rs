// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! KMS over PDX: the messages a display client exchanges with a display driver
//! service. Each call carries four words; larger answers are paged by index.
//! Opcodes 0x2F0..=0x2FF belong to KMS.

use crate::commit::Change;
use crate::mode::Mode;
use crate::model::Rect;

/// → (connectors, crtcs, planes, generation). Generation bumps on hotplug.
pub const OP_KMS_INFO: u64 = 0x2F0;
/// rdi=connector → kind | number<<8 | status<<16 | modes<<24 | width_mm<<40, height_mm, possible_crtcs.
pub const OP_KMS_CONNECTOR: u64 = 0x2F1;
/// rdi=connector, rsi=mode index → the mode as [`pack_mode`].
pub const OP_KMS_MODE: u64 = 0x2F2;
/// Start a commit; changes queue until [`OP_KMS_COMMIT`].
pub const OP_KMS_BEGIN: u64 = 0x2F3;
/// Four words of one [`Change`] as [`pack_change`].
pub const OP_KMS_CHANGE: u64 = 0x2F4;
/// rdi=flags → 0, or an error code. Applies the queued changes atomically.
pub const OP_KMS_COMMIT: u64 = 0x2F5;
/// rdi=width, rsi=height, rdx=stride, r10=format → framebuffer id (memory lent separately).
pub const OP_KMS_ADD_FB: u64 = 0x2F6;
pub const OP_KMS_REMOVE_FB: u64 = 0x2F7;
/// → crtc, sequence, time_us of the oldest pending flip event, or ERR_NONE.
pub const OP_KMS_EVENT: u64 = 0x2F8;
pub const ERR_NONE: u64 = u64::MAX;

pub fn pack_mode(m: &Mode) -> [u64; 4] {
    [m.clock_khz as u64 | (m.hdisplay as u64) << 32 | (m.hsync_start as u64) << 48,
     m.hsync_end as u64 | (m.htotal as u64) << 16 | (m.vdisplay as u64) << 32 | (m.vsync_start as u64) << 48,
     m.vsync_end as u64 | (m.vtotal as u64) << 16 | (m.flags as u64) << 32,
     0]
}
pub fn unpack_mode(w: [u64; 4]) -> Mode {
    Mode { clock_khz: w[0] as u32, hdisplay: (w[0] >> 32) as u16, hsync_start: (w[0] >> 48) as u16,
        hsync_end: w[1] as u16, htotal: (w[1] >> 16) as u16, vdisplay: (w[1] >> 32) as u16, vsync_start: (w[1] >> 48) as u16,
        vsync_end: w[2] as u16, vtotal: (w[2] >> 16) as u16, flags: (w[2] >> 32) as u8 }
}

const NONE8: u64 = 0xff;
const NONE16: u64 = 0xffff;
fn opt8(v: Option<u32>) -> u64 { v.map_or(NONE8, |x| x as u64 & 0xff) }
fn opt16(v: Option<u32>) -> u64 { v.map_or(NONE16, |x| x as u64 & 0xffff) }
fn un8(v: u64) -> Option<u32> { let v = v & 0xff; if v == NONE8 { None } else { Some(v as u32) } }
fn un16(v: u64) -> Option<u32> { let v = v & 0xffff; if v == NONE16 { None } else { Some(v as u32) } }
fn rect(r: Rect) -> u64 { (r.x as u16) as u64 | ((r.y as u16) as u64) << 16 | (r.w as u64 & 0xffff) << 32 | (r.h as u64 & 0xffff) << 48 }
fn unrect(v: u64) -> Rect { Rect::new(v as u16 as i16 as i32, (v >> 16) as u16 as i16 as i32, (v >> 32) as u16 as u32, (v >> 48) as u16 as u32) }

/// One change in four words. Word 0 low byte is the kind. Modes travel as the
/// first three [`pack_mode`] words with the kind and CRTC in word 3.
pub fn pack_change(c: &Change) -> [u64; 4] {
    match *c {
        Change::Route { connector, crtc } => [1 | (connector as u64 & 0xff) << 8 | opt8(crtc) << 16, 0, 0, 0],
        Change::Active { crtc, on } => [2 | (crtc as u64 & 0xff) << 8 | (on as u64) << 16, 0, 0, 0],
        Change::Mode { crtc, mode } => match mode {
            Some(m) => { let w = pack_mode(&m); [w[0], w[1], w[2], 3 | (crtc as u64 & 0xff) << 8 | 1 << 16] }
            None => [0, 0, 0, 3 | (crtc as u64 & 0xff) << 8],
        },
        Change::Plane { plane, crtc, fb, src, dst } =>
            [4 | (plane as u64 & 0xff) << 8 | opt8(crtc) << 16 | opt16(fb) << 24, rect(src), rect(dst), 0],
        Change::Flip { plane, fb } => [5 | (plane as u64 & 0xff) << 8 | (fb as u64 & 0xffff) << 24, 0, 0, 0],
        Change::Move { plane, x, y } => [6 | (plane as u64 & 0xff) << 8, (x as u32) as u64 | ((y as u32) as u64) << 32, 0, 0],
    }
}
pub fn unpack_change(w: [u64; 4]) -> Option<Change> {
    if w[3] & 0xff == 3 {
        let crtc = (w[3] >> 8) as u8 as u32;
        let mode = if (w[3] >> 16) & 1 != 0 { Some(unpack_mode([w[0], w[1], w[2], 0])) } else { None };
        return Some(Change::Mode { crtc, mode });
    }
    let id = (w[0] >> 8) as u8 as u32;
    Some(match w[0] & 0xff {
        1 => Change::Route { connector: id, crtc: un8(w[0] >> 16) },
        2 => Change::Active { crtc: id, on: (w[0] >> 16) & 1 != 0 },
        4 => Change::Plane { plane: id, crtc: un8(w[0] >> 16), fb: un16(w[0] >> 24), src: unrect(w[1]), dst: unrect(w[2]) },
        5 => Change::Flip { plane: id, fb: ((w[0] >> 24) & 0xffff) as u32 },
        6 => Change::Move { plane: id, x: w[1] as u32 as i32, y: (w[1] >> 32) as u32 as i32 },
        _ => return None,
    })
}
