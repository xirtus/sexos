// SPDX-License-Identifier: GPL-3.0-only
//! USB sound cards (USB Audio Class 1 and 2), between Bell and SexUSB: Bell's mix goes
//! out through one shared region.
//!
//! BOOT gives Bell and SexUSB the same region at [`SLOT`]. SexUSB drives the card and
//! writes into the header whether one is plugged in and what it is; Bell writes 48 kHz
//! stereo S16 frames into the output ring and SexUSB sends them to the card, one
//! service interval at a time. The ring has one producer and one consumer and both
//! sides poll it: a producer writes frames, then publishes its count (release); a
//! consumer reads the count (acquire), then the frames, then publishes its own count.
//! Counts are frames since the card appeared and never wrap in practice (u64).
//!
//! A microphone (a headset's, or a USB microphone on its own) goes the other way
//! through the capture ring: Bell says when someone may listen, SexUSB runs the
//! card's input then and writes what it hears as 48 kHz stereo S16. A microphone is
//! announced apart from playback, so a USB microphone works next to the laptop's
//! speakers. When Bell falls behind, the oldest frames are lost, never the newest.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// The shared region, in Bell and in SexUSB.
pub const SLOT: u64 = 1009;
pub const REGION_ID: u32 = 0x4406;
pub const BYTES: u64 = 128 * 1024;
/// Frames the output ring holds (S16 stereo, 4 bytes each): about 340 ms.
pub const FRAMES: u64 = 16 * 1024;
pub const RATE: u32 = 48_000;
const MAGIC: u32 = 0x5553_4141; // "AASU"
const HEADER: usize = 4096;
const OUT: usize = HEADER;
/// Frames the capture ring holds: about 170 ms.
pub const MIC_FRAMES: u64 = 8 * 1024;
const MIC: usize = OUT + FRAMES as usize * 4;
const _: () = assert!(MIC as u64 + MIC_FRAMES * 4 <= BYTES);

// Header words (byte offsets).
const W_MAGIC: usize = 0;
const W_LINK: usize = 4;
const W_VENDOR: usize = 8;
const W_PRODUCT: usize = 12;
const W_GENERATION: usize = 16;
const W_CLASS: usize = 20;
const W_UNDERRUNS: usize = 24;
/// SexUSB: a microphone is there (its generation, 0 for none); Bell: may listen (1).
const W_MIC: usize = 28;
const W_MIC_VENDOR: usize = 32;
const W_MIC_PRODUCT: usize = 36;
const W_LISTEN: usize = 40;
const W_MIC_GENERATION: usize = 44;
const D_WRITTEN: usize = 64;
const D_TAKEN: usize = 72;
/// Frames SexUSB has heard, and frames Bell has read.
const D_HEARD: usize = 80;
const D_READ: usize = 88;

/// What SexUSB says about the card plugged in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Card { pub vendor: u16, pub product: u16, pub class: u8, pub generation: u32 }

/// The region, seen from either side. `base` must be [`BYTES`] long and 8-byte aligned.
#[derive(Clone, Copy)]
pub struct Shared { base: *mut u8 }

impl Shared {
    /// # Safety
    /// `base` points at a live mapping of at least [`BYTES`] bytes for as long as this is used.
    pub unsafe fn at(base: *mut u8) -> Shared { Shared { base } }
    fn word(&self, at: usize) -> &AtomicU32 { unsafe { &*(self.base.add(at) as *const AtomicU32) } }
    fn dword(&self, at: usize) -> &AtomicU64 { unsafe { &*(self.base.add(at) as *const AtomicU64) } }
    fn load(&self, at: usize) -> u32 { self.word(at).load(Ordering::Acquire) }
    fn store(&self, at: usize, v: u32) { self.word(at).store(v, Ordering::Release) }
    fn frames(&self) -> *mut [i16; 2] { unsafe { self.base.add(OUT) as *mut [i16; 2] } }

    /// SexUSB: a card is ready to play (`Some`) or gone (`None`). The ring starts empty.
    pub fn set_card(&self, card: Option<(u16, u16, u8)>) {
        self.store(W_LINK, 0);
        self.dword(D_WRITTEN).store(0, Ordering::Release);
        self.dword(D_TAKEN).store(0, Ordering::Release);
        self.store(W_UNDERRUNS, 0);
        if let Some((vendor, product, class)) = card {
            self.store(W_VENDOR, vendor as u32);
            self.store(W_PRODUCT, product as u32);
            self.store(W_CLASS, class as u32);
            self.store(W_GENERATION, self.load(W_GENERATION).wrapping_add(1));
            self.store(W_MAGIC, MAGIC);
            self.store(W_LINK, 1);
        }
    }
    /// Either side: the card plugged in now, if any.
    pub fn card(&self) -> Option<Card> {
        if self.load(W_MAGIC) != MAGIC || self.load(W_LINK) != 1 { return None; }
        Some(Card { vendor: self.load(W_VENDOR) as u16, product: self.load(W_PRODUCT) as u16,
            class: self.load(W_CLASS) as u8, generation: self.load(W_GENERATION) })
    }
    /// Frames Bell has written, and frames SexUSB has taken.
    pub fn written(&self) -> u64 { self.dword(D_WRITTEN).load(Ordering::Acquire) }
    pub fn taken(&self) -> u64 { self.dword(D_TAKEN).load(Ordering::Acquire) }
    /// Bell: frames there is room for.
    pub fn room(&self) -> u64 { FRAMES - self.written().wrapping_sub(self.taken()).min(FRAMES) }
    /// Bell: queue as many of `frames` as fit; returns how many did.
    pub fn write(&self, frames: &[[i16; 2]]) -> usize {
        let n = (frames.len() as u64).min(self.room()) as usize;
        let w = self.written();
        let base = self.frames();
        for (k, f) in frames[..n].iter().enumerate() {
            unsafe { core::ptr::write_volatile(base.add(((w + k as u64) % FRAMES) as usize), *f) };
        }
        self.dword(D_WRITTEN).store(w + n as u64, Ordering::Release);
        n
    }
    /// SexUSB: take up to `out.len()` frames into `out`; the rest of `out` is silence.
    /// Returns how many were real sound (fewer than asked counts an underrun once the
    /// stream has started).
    pub fn take(&self, out: &mut [[i16; 2]]) -> usize {
        let (w, t) = (self.written(), self.taken());
        let have = w.wrapping_sub(t).min(FRAMES) as usize;
        let n = have.min(out.len());
        let base = self.frames();
        for (k, f) in out.iter_mut().enumerate() {
            *f = if k < n { unsafe { core::ptr::read_volatile(base.add(((t + k as u64) % FRAMES) as usize)) } } else { [0, 0] };
        }
        self.dword(D_TAKEN).store(t + n as u64, Ordering::Release);
        if n < out.len() && n > 0 { self.store(W_UNDERRUNS, self.load(W_UNDERRUNS).wrapping_add(1)); }
        n
    }
    pub fn underruns(&self) -> u32 { self.load(W_UNDERRUNS) }

    fn mic_frames(&self) -> *mut [i16; 2] { unsafe { self.base.add(MIC) as *mut [i16; 2] } }
    /// SexUSB: a microphone is ready (`Some((vendor, product))`) or gone. The capture
    /// ring starts empty; whether Bell listens is Bell's and stays as it was.
    pub fn set_mic(&self, mic: Option<(u16, u16)>) {
        self.store(W_MIC, 0);
        self.dword(D_HEARD).store(0, Ordering::Release);
        self.dword(D_READ).store(0, Ordering::Release);
        if let Some((vendor, product)) = mic {
            self.store(W_MIC_VENDOR, vendor as u32);
            self.store(W_MIC_PRODUCT, product as u32);
            let g = self.load(W_MIC_GENERATION).wrapping_add(1).max(1);
            self.store(W_MIC_GENERATION, g);
            self.store(W_MAGIC, MAGIC);
            self.store(W_MIC, g);
        }
    }
    /// Either side: the microphone plugged in now (vendor, product, generation).
    pub fn mic(&self) -> Option<(u16, u16, u32)> {
        let g = self.load(W_MIC);
        (self.load(W_MAGIC) == MAGIC && g != 0)
            .then(|| (self.load(W_MIC_VENDOR) as u16, self.load(W_MIC_PRODUCT) as u16, g))
    }
    /// Bell: someone may listen (the card's input runs), or no one does.
    pub fn set_listen(&self, on: bool) { self.store(W_LISTEN, on as u32); }
    pub fn listen(&self) -> bool { self.load(W_LISTEN) == 1 }
    /// SexUSB: frames the microphone heard.
    pub fn hear(&self, frames: &[[i16; 2]]) {
        let h = self.dword(D_HEARD).load(Ordering::Acquire);
        let base = self.mic_frames();
        for (k, f) in frames.iter().enumerate() {
            unsafe { core::ptr::write_volatile(base.add(((h + k as u64) % MIC_FRAMES) as usize), *f) };
        }
        self.dword(D_HEARD).store(h + frames.len() as u64, Ordering::Release);
    }
    /// Bell: frames heard and not yet read (at most the ring's worth).
    pub fn heard(&self) -> u64 {
        self.dword(D_HEARD).load(Ordering::Acquire).wrapping_sub(self.dword(D_READ).load(Ordering::Acquire)).min(MIC_FRAMES)
    }
    /// Bell: read up to `out.len()` heard frames, the oldest still kept first; returns
    /// how many. Frames SexUSB wrote over while being read are dropped, never mixed.
    pub fn read_heard(&self, out: &mut [[i16; 2]]) -> usize {
        let h = self.dword(D_HEARD).load(Ordering::Acquire);
        // Keep a margin: SexUSB may be writing the next packet over the oldest frames.
        let keep = MIC_FRAMES - MIC_FRAMES / 8;
        let r = self.dword(D_READ).load(Ordering::Acquire).max(h.saturating_sub(keep));
        let n = (h - r.min(h)).min(out.len() as u64) as usize;
        let base = self.mic_frames();
        for (k, f) in out[..n].iter_mut().enumerate() {
            *f = unsafe { core::ptr::read_volatile(base.add(((r + k as u64) % MIC_FRAMES) as usize)) };
        }
        self.dword(D_READ).store(r + n as u64, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_go_round() {
        let mut mem = vec![0u64; BYTES as usize / 8];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert_eq!(s.card(), None);
        s.set_card(Some((0x46f4, 0x0002, 1)));
        assert_eq!(s.card().map(|c| (c.vendor, c.product, c.class, c.generation)), Some((0x46f4, 2, 1, 1)));
        let frames: Vec<[i16; 2]> = (0..FRAMES as i16 + 10).map(|k| [k, -k]).collect();
        assert_eq!(s.write(&frames), FRAMES as usize);
        assert_eq!(s.room(), 0);
        let mut out = [[0i16; 2]; 48];
        assert_eq!(s.take(&mut out), 48);
        assert_eq!(out[47], [47, -47]);
        assert_eq!(s.write(&frames[100..]), 48);
        let mut all = vec![[0i16; 2]; FRAMES as usize + 4];
        assert_eq!(s.take(&mut all), FRAMES as usize);
        assert_eq!(all[FRAMES as usize - 1], [147, -147]);
        assert_eq!(all[FRAMES as usize], [0, 0]);
        assert_eq!(s.underruns(), 1);
        s.set_card(None);
        assert_eq!(s.card(), None);
    }

    #[test]
    fn microphone_frames_come_back_newest_kept() {
        let mut mem = vec![0u64; BYTES as usize / 8];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert_eq!(s.mic(), None);
        s.set_listen(true);
        s.set_mic(Some((0x0d8c, 0x0014)));
        assert_eq!(s.mic(), Some((0x0d8c, 0x0014, 1)));
        assert!(s.listen(), "listening is Bell's and survives a new microphone");
        assert_eq!(s.card(), None, "a microphone alone is no playback card");
        let frames: Vec<[i16; 2]> = (0..100).map(|k| [k, k]).collect();
        s.hear(&frames);
        let mut out = [[0i16; 2]; 64];
        assert_eq!(s.read_heard(&mut out), 64);
        assert_eq!(out[63], [63, 63]);
        assert_eq!(s.heard(), 36);
        // Bell falls far behind: the oldest are lost, the newest kept.
        let many: Vec<[i16; 2]> = (0..MIC_FRAMES as i32 * 2).map(|k| [k as i16, 0]).collect();
        s.hear(&many);
        let mut all = vec![[0i16; 2]; MIC_FRAMES as usize];
        let n = s.read_heard(&mut all);
        assert_eq!(n as u64, MIC_FRAMES - MIC_FRAMES / 8);
        assert_eq!(all[n - 1][0], (MIC_FRAMES as i32 * 2 - 1) as i16);
        assert_eq!(s.heard(), 0);
        s.set_mic(None);
        assert_eq!(s.mic(), None);
        s.set_mic(Some((1, 2)));
        assert_eq!(s.mic().map(|m| m.2), Some(2));
    }
}
