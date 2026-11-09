// SPDX-License-Identifier: GPL-3.0-only
//! Sound for programs: frames go into the program's own sound ring and Bell mixes
//! them into the sound card, so a program never touches the card or another
//! program's sound. 48 kHz, stereo, i16.
use sex_pdx::sound_ring::{self as ring, Ring};

pub use ring::RATE;

pub struct Sound { ring: Ring, written: u64, start: u64, told: bool }

impl Sound {
    /// The program's ring, or `None` when this program row has none.
    pub fn open() -> Option<Sound> {
        let r = sex_pdx::sound_ring_client().ok()?;
        // Start where Bell left off: a ring reused by a new run keeps its counters.
        let taken = r.load(ring::TAKEN);
        r.store(ring::WRITTEN, taken);
        r.store(ring::VOLUME, 100);
        r.store(ring::OPEN, 1);
        Some(Sound { ring: r, written: taken, start: taken, told: false })
    }
    /// Frames there is room for now.
    pub fn room(&self) -> usize { (ring::FRAMES - self.written.wrapping_sub(self.ring.load(ring::TAKEN)).min(ring::FRAMES)) as usize }
    /// Queue as many of `frames` as fit; returns how many did.
    pub fn write(&mut self, frames: &[[i16; 2]]) -> usize {
        let n = frames.len().min(self.room());
        let base = self.ring.frames();
        for (i, f) in frames[..n].iter().enumerate() {
            let at = ((self.written + i as u64) % ring::FRAMES) as usize;
            unsafe { core::ptr::write_volatile(base.add(at), *f) };
        }
        self.written += n as u64;
        self.ring.store(ring::WRITTEN, self.written);
        if n > 0 { self.kick(); }
        n
    }
    /// Tell Bell there is sound: it plays until the ring runs dry and the program closes.
    pub fn kick(&mut self) {
        // Bell keeps an open stream running, so once is enough until it closes.
        if !self.told || !self.playing() {
            let _ = sex_pdx::pdx_call(sex_pdx::SLOT_BELL, ring::OP_PLAY, 0, 0, 0);
            self.told = true;
        }
    }
    /// Whether Bell still has frames of ours it has not played.
    pub fn playing(&self) -> bool { self.ring.load(ring::PLAYED) < self.written }
    /// Frames queued since `open`.
    pub fn written(&self) -> u64 { self.written - self.start }
    /// Frames the card has played since `open`; the program's sound clock. The ring
    /// itself counts from when it began, across runs of the program.
    pub fn played(&self) -> u64 { self.ring.load(ring::PLAYED).saturating_sub(self.start) }
    /// Frames written but not yet heard.
    pub fn pending(&self) -> u64 { self.written.saturating_sub(self.ring.load(ring::PLAYED)) }
    pub fn set_volume(&mut self, percent: u8) { self.ring.store(ring::VOLUME, percent.min(100) as u64); }
    /// Stop: Bell drops the stream once it has played what is queued.
    pub fn close(&mut self) { self.ring.store(ring::OPEN, 0); }
}

impl Drop for Sound {
    fn drop(&mut self) { self.close(); }
}
