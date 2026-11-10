// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A sound ring for each program row: a BOOT region the program fills with sound and
//! Bell plays, mixed with every other program's and with Bell's own notes. Holding it
//! grants only that row's sound; Bell takes a stream only from the row that owns it.
//!
//! Layout: a 4 KiB header, then a ring of frames. A frame is one 48 kHz stereo sample
//! pair of little-endian i16. Counters only grow; position = counter % FRAMES.
//! - `WRITTEN` (program writes): frames the program has put in the ring.
//! - `TAKEN` (Bell writes): frames Bell has copied out to the card.
//! - `PLAYED` (Bell writes): frames the card has finished playing, the program's clock.
//! - `OPEN` (program writes): 1 while the program means to keep playing. Bell drops
//!   a stream that is closed and drained.
//! - `VOLUME` (program writes): 0..=100, the stream's loudness in percent.
//! - `PAUSED` (program writes): nonzero holds the stream where it is; Bell takes
//!   nothing until it is zero again.
//! - `SKIP_TO` (program writes): a WRITTEN count. Bell drops what lies before it
//!   unplayed and counts it as played, so a program that seeks loses its queued sound
//!   at once.
use crate::app_catalog::{App, INCARNATIONS};

pub const RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;
pub const FRAME_BYTES: u64 = 4;
pub const BYTES: u64 = 256 * 1024;
pub const HEADER_BYTES: u64 = 4096;
pub const FRAMES: u64 = (BYTES - HEADER_BYTES) / FRAME_BYTES;

pub const WRITTEN: usize = 0;
pub const TAKEN: usize = 8;
pub const PLAYED: usize = 16;
pub const OPEN: usize = 24;
pub const VOLUME: usize = 32;
pub const PAUSED: usize = 40;
pub const SKIP_TO: usize = 48;
/// The rate the program's frames are at, in Hz (8000..=192000); 0 means [`RATE`].
/// Bell converts to its own rate as it mixes.
pub const RATE_HZ: usize = 56;

/// The program's own view, writable.
pub const CLIENT_SLOT: u64 = 260;
/// Bell's views, one per program row; Bell writes the counters it owns.
pub const BELL_FIRST_SLOT: u64 = 900;
pub const fn bell_slot(index: usize) -> u64 { BELL_FIRST_SLOT + index as u64 }
pub const fn region_id(index: usize) -> u32 { 0x4800 + index as u32 }
/// Program to Bell (`SLOT_BELL`): there is sound in my ring; play it. No arguments.
pub const OP_PLAY: u64 = 0xC9;
/// The program row's position among program rows.
pub fn index_of(target: u32) -> Option<usize> {
    INCARNATIONS.iter().filter(|r| r.app == App::Program).position(|r| r.target == target)
}

/// One side's view of a ring.
#[derive(Clone, Copy)]
pub struct Ring { base: *mut u8 }

impl Ring {
    /// # Safety
    /// `base` must be a live view of a whole ring of `BYTES`.
    pub const unsafe fn at(base: *mut u8) -> Ring { Ring { base } }
    fn word(&self, at: usize) -> *mut u64 { unsafe { self.base.add(at) as *mut u64 } }
    pub fn load(&self, at: usize) -> u64 {
        let v = unsafe { core::ptr::read_volatile(self.word(at)) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        v
    }
    pub fn store(&self, at: usize, value: u64) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        unsafe { core::ptr::write_volatile(self.word(at), value) }
    }
    /// The frame slots, as stereo pairs.
    pub fn frames(&self) -> *mut [i16; 2] { unsafe { self.base.add(HEADER_BYTES as usize) as *mut [i16; 2] } }
}
