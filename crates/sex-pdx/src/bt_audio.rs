// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bluetooth audio between Bell (the sound server) and Cufflinks (the Bluetooth host):
//! while a headset is connected and ready for a stream, Bell puts its whole mix (every
//! program's stream, its own notes) in this region instead of on the speakers, and
//! Cufflinks encodes it (SBC) and sends it to the headset.
//!
//! One region, paired by BOOT between Bell and Cufflinks (kernel
//! `boot_memory::init_bluetooth`), no routes and no new operations: each side polls
//! it, Bell as it renders a period for its card, Cufflinks every tick while a headset
//! is up. Layout: a 4 KiB header of u32 words, then a ring of [`FRAMES`] stereo frames
//! (little-endian i16 pairs at [`RATE`] Hz). Counters only grow (wrapping); a frame's
//! position is counter % `FRAMES`.
//!
//! - `SINK` (Cufflinks writes): nonzero while a headset takes audio; Bell then sends
//!   its mix here and silence to the card. Bell keeps its card's clock, so the card
//!   still paces the mix (with no sound card Bell renders nothing; see the W3 notes).
//! - `WRITTEN` (Bell): frames put in the ring. Bell never overwrites what Cufflinks
//!   has not taken: frames that do not fit are dropped and counted in `DROPPED`.
//! - `TAKEN` (Cufflinks): frames taken out.
//! - `SINKS` (Cufflinks): how many times a headset became ready, for the log.
//! - `MASTER` (Bell): the master volume, [`MASTER_VALID`] | muted << 8 | volume
//!   (0..=100), written whenever Bell answers a mixer request, so Cufflinks can mirror
//!   it to a headset that does AVRCP absolute volume.
//! - `HEADSET_VOLUME` (Cufflinks): nonzero while that headset applies the volume
//!   itself; Bell then leaves the master volume out of the mix it sends here (mute
//!   still silences it), so the volume is not applied twice.

use core::sync::atomic::{AtomicU32, Ordering};

/// The region, in Bell and in Cufflinks.
pub const SLOT: u64 = 1006;
pub const REGION_ID: u32 = 0x4404;
pub const BYTES: u64 = 256 * 1024;
pub const HEADER_BYTES: usize = 4096;
/// Frames the ring holds: 64512, 1.3 s at 48 kHz (Bell renders up to about half a
/// second ahead of its card; Cufflinks paces the headset by its own clock).
pub const FRAMES: u32 = ((BYTES as usize - HEADER_BYTES) / 4) as u32;
pub const RATE: u32 = 48_000;

const MAGIC: u32 = 0x4F44_5542; // "BUDO"
const W_MAGIC: usize = 0;
const W_SINK: usize = 4;
const W_WRITTEN: usize = 8;
const W_TAKEN: usize = 12;
const W_DROPPED: usize = 16;
const W_SINKS: usize = 20;
const W_MASTER: usize = 24;
const W_HEADSET_VOLUME: usize = 28;
/// Set in `MASTER` once Bell has written it (a zeroed region says nothing).
pub const MASTER_VALID: u32 = 1 << 31;

/// The region, seen from either side. `base` must be [`BYTES`] long and 4-byte aligned.
#[derive(Clone, Copy)]
pub struct Shared { base: *mut u8 }

impl Shared {
    /// # Safety
    /// `base` points at a live mapping of at least [`BYTES`] bytes for as long as this is used.
    pub unsafe fn at(base: *mut u8) -> Shared { Shared { base } }
    fn word(&self, at: usize) -> &AtomicU32 { unsafe { &*(self.base.add(at) as *const AtomicU32) } }
    fn load(&self, at: usize) -> u32 { self.word(at).load(Ordering::Acquire) }
    fn store(&self, at: usize, v: u32) { self.word(at).store(v, Ordering::Release) }
    fn frame(&self, index: u32) -> *mut [i16; 2] { unsafe { (self.base.add(HEADER_BYTES) as *mut [i16; 2]).add((index % FRAMES) as usize) } }

    /// Cufflinks: a headset is ready for audio (true) or gone (false). Turning the sink
    /// on starts an empty ring.
    pub fn set_sink(&self, on: bool) {
        if on {
            self.store(W_TAKEN, self.load(W_WRITTEN));
            self.store(W_SINKS, self.load(W_SINKS).wrapping_add(1));
            self.store(W_MAGIC, MAGIC);
        }
        self.store(W_SINK, on as u32);
    }
    /// Bell: whether its mix should go to a headset now.
    pub fn sink(&self) -> bool { self.load(W_MAGIC) == MAGIC && self.load(W_SINK) != 0 }
    pub fn sinks(&self) -> u32 { self.load(W_SINKS) }

    /// Bell: the master volume (0..=100) and mute, as the person set them.
    pub fn set_master(&self, volume: u8, muted: bool) { self.store(W_MASTER, MASTER_VALID | (muted as u32) << 8 | volume.min(100) as u32); }
    /// Cufflinks: the master volume and mute, once Bell has written them.
    pub fn master(&self) -> Option<(u8, bool)> {
        let w = self.load(W_MASTER);
        (w & MASTER_VALID != 0).then(|| (((w & 0xFF) as u8).min(100), w & 1 << 8 != 0))
    }
    /// Cufflinks: the headset applies the master volume itself (true) or not.
    pub fn set_headset_volume(&self, on: bool) { self.store(W_HEADSET_VOLUME, on as u32); }
    /// Bell: whether the mix going here should leave the master volume out (only while
    /// the sink is on).
    pub fn headset_volume(&self) -> bool { self.sink() && self.load(W_HEADSET_VOLUME) != 0 }
    pub fn dropped(&self) -> u32 { self.load(W_DROPPED) }

    /// Frames waiting for Cufflinks. A `TAKEN` ahead of `WRITTEN` or more than a ring
    /// behind (a side that went wrong) counts as empty or full, never more.
    pub fn queued(&self) -> u32 { self.load(W_WRITTEN).wrapping_sub(self.load(W_TAKEN)).min(FRAMES) }

    /// Bell: put frames in the ring; returns how many fit (the rest are dropped and counted).
    pub fn push(&self, frames: &[[i16; 2]]) -> usize {
        let w = self.load(W_WRITTEN);
        let used = w.wrapping_sub(self.load(W_TAKEN));
        let room = if used > FRAMES { 0 } else { (FRAMES - used) as usize };
        let n = frames.len().min(room);
        for (i, f) in frames[..n].iter().enumerate() { unsafe { self.frame(w.wrapping_add(i as u32)).write_volatile(*f) } }
        self.store(W_WRITTEN, w.wrapping_add(n as u32));
        if n < frames.len() { self.store(W_DROPPED, self.load(W_DROPPED).wrapping_add((frames.len() - n) as u32)); }
        n
    }
    /// Bell: put a period of little-endian i16 stereo bytes in the ring (as `push`).
    pub fn push_bytes(&self, bytes: &[u8]) -> usize {
        let mut pairs = [[0i16; 2]; 256];
        let mut done = 0;
        for chunk in bytes.chunks(256 * 4) {
            let n = chunk.len() / 4;
            for (i, f) in chunk.chunks_exact(4).enumerate() {
                pairs[i] = [i16::from_le_bytes([f[0], f[1]]), i16::from_le_bytes([f[2], f[3]])];
            }
            let took = self.push(&pairs[..n]);
            done += took;
            if took < n {
                // The rest of the period is dropped too.
                let rest = (bytes.len() / 4).saturating_sub(done + (n - took));
                self.store(W_DROPPED, self.load(W_DROPPED).wrapping_add(rest as u32));
                break;
            }
        }
        done
    }
    /// Cufflinks: copy up to `out.len()` waiting frames without taking them.
    pub fn peek(&self, out: &mut [[i16; 2]]) -> usize {
        let t = self.load(W_TAKEN);
        let n = (self.queued() as usize).min(out.len());
        for (i, o) in out[..n].iter_mut().enumerate() { *o = unsafe { self.frame(t.wrapping_add(i as u32)).read_volatile() }; }
        n
    }
    /// Cufflinks: take `n` frames (at most what is waiting).
    pub fn consume(&self, n: usize) {
        let n = (n as u32).min(self.queued());
        self.store(W_TAKEN, self.load(W_TAKEN).wrapping_add(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring() {
        let mut mem = vec![0u32; BYTES as usize / 4];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(!s.sink());
        s.set_sink(true);
        assert!(s.sink() && s.sinks() == 1);
        let frames: Vec<[i16; 2]> = (0..1000).map(|i| [i as i16, -(i as i16)]).collect();
        assert_eq!(s.push(&frames), 1000);
        let mut out = [[0i16; 2]; 600];
        assert_eq!(s.peek(&mut out), 600);
        assert_eq!(out[599], [599, -599]);
        s.consume(600);
        assert_eq!(s.peek(&mut out), 400);
        assert_eq!(out[0], [600, -600]);
        s.consume(9999);
        assert_eq!(s.queued(), 0);
        // Full: the rest is dropped, never written over what waits.
        let big = vec![[7i16, 7]; FRAMES as usize + 10];
        assert_eq!(s.push(&big), FRAMES as usize);
        assert_eq!(s.dropped(), 10);
        assert_eq!(s.push(&frames), 0);
        // Wrapping counters and a re-armed sink start empty.
        s.consume(FRAMES as usize);
        assert_eq!(s.push(&frames), 1000);
        s.set_sink(false);
        assert!(!s.sink());
        s.set_sink(true);
        assert_eq!((s.queued(), s.sinks()), (0, 2));
        // Bell's period bytes.
        let bytes: Vec<u8> = [[1i16, -2], [300, -400]].iter().flat_map(|f| [f[0].to_le_bytes(), f[1].to_le_bytes()].concat()).collect();
        assert_eq!(s.push_bytes(&bytes), 2);
        let mut two = [[0i16; 2]; 2];
        assert_eq!(s.peek(&mut two), 2);
        assert_eq!(two, [[1, -2], [300, -400]]);
        // The master volume, and the headset applying it, only while the sink is on.
        assert_eq!(s.master(), None);
        s.set_master(250, true);
        assert_eq!(s.master(), Some((100, true)));
        s.set_master(35, false);
        assert_eq!(s.master(), Some((35, false)));
        s.set_headset_volume(true);
        assert!(s.headset_volume());
        s.set_sink(false);
        assert!(!s.headset_volume());
    }
}
