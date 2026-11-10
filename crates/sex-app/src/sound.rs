// SPDX-License-Identifier: GPL-3.0-only
//! Sound for programs: frames go into the program's own sound ring and Bell mixes
//! them into the sound card, so a program never touches the card or another
//! program's sound. Stereo i16 frames, at 48 kHz unless the program names another
//! rate (`set_rate`): Bell converts as it mixes.
//!
//! The microphone comes the same way, the other direction: `listen(true)` asks, and
//! once the person allows it (`volume ROW mic on`) Bell copies what it hears into the
//! ring for `hear` to read. Without that leave the program hears nothing.
use sex_pdx::sound_ring::{self as ring, Ring};

pub use ring::RATE;

pub struct Sound { ring: Ring, written: u64, start: u64, told: bool, heard: u64, lost: u64, listening: bool }

impl Sound {
    /// The program's ring, or `None` when this program row has none.
    pub fn open() -> Option<Sound> {
        let r = sex_pdx::sound_ring_client().ok()?;
        // Start where Bell left off: a ring reused by a new run keeps its counters.
        let taken = r.load(ring::TAKEN);
        r.store(ring::WRITTEN, taken);
        r.store(ring::VOLUME, 100);
        r.store(ring::PAUSED, 0);
        r.store(ring::SKIP_TO, taken);
        r.store(ring::RATE_HZ, 0);
        r.store(ring::OPEN, 1);
        r.store(ring::LISTEN, 0);
        Some(Sound { ring: r, written: taken, start: taken, told: false, heard: 0, lost: 0, listening: false })
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
    /// Hold the sound where it is, or go on. Paused sound stays queued.
    pub fn pause(&mut self, paused: bool) {
        self.ring.store(ring::PAUSED, paused as u64);
        if !paused { self.told = false; self.kick(); }
    }
    /// Drop everything queued and not yet played; it counts as played at once (up to
    /// what Bell already handed the card, about half a second, which still plays).
    pub fn skip(&mut self) { self.ring.store(ring::SKIP_TO, self.written); self.kick_now(); }
    /// The ring's own count of frames written, which `played_total` reaches once all
    /// of them have played.
    pub fn written_total(&self) -> u64 { self.written }
    /// The ring's own count of frames played, across runs of the program.
    pub fn played_total(&self) -> u64 { self.ring.load(ring::PLAYED) }
    fn kick_now(&mut self) { self.told = false; self.kick(); }
    /// The rate of the frames written from now on, 8000 to 192000 Hz (else 48000).
    pub fn set_rate(&mut self, hz: u32) { self.ring.store(ring::RATE_HZ, hz as u64); }
    pub fn set_volume(&mut self, percent: u8) { self.ring.store(ring::VOLUME, percent.min(100) as u64); }
    /// Undo `close`: the program means to play more into this ring.
    pub fn reopen(&mut self) { self.ring.store(ring::OPEN, 1); self.told = false; }
    /// Ask to hear the microphone, or stop. Only frames heard from now on are read.
    pub fn listen(&mut self, on: bool) {
        self.heard = self.ring.load(ring::HEARD);
        self.ring.store(ring::LISTEN, on as u64);
        self.listening = on;
        let _ = sex_pdx::pdx_call(sex_pdx::SLOT_BELL, ring::OP_LISTEN, 0, 0, 0);
    }
    /// Where listening stands: `ring::MIC_OFF`, `MIC_WAITING` (the person has not
    /// allowed it), `MIC_ON` or `MIC_NONE` (no microphone).
    pub fn mic(&self) -> u64 { self.ring.load(ring::MIC) }
    /// Microphone frames (48 kHz stereo) heard since the last call, up to `out.len()`.
    /// Frames the ring no longer holds because the program fell behind are skipped and
    /// counted in `lost`.
    pub fn hear(&mut self, out: &mut [[i16; 2]]) -> usize {
        let now = self.ring.load(ring::HEARD);
        if now.wrapping_sub(self.heard) > ring::CAPTURE_FRAMES {
            let from = now - ring::CAPTURE_FRAMES;
            self.lost += from.wrapping_sub(self.heard);
            self.heard = from;
        }
        let n = (now.wrapping_sub(self.heard) as usize).min(out.len());
        let slots = self.ring.heard();
        for (i, f) in out[..n].iter_mut().enumerate() {
            *f = unsafe { core::ptr::read_volatile(slots.add(((self.heard + i as u64) % ring::CAPTURE_FRAMES) as usize)) };
        }
        self.heard += n as u64;
        n
    }
    /// Microphone frames lost because the program read too slowly.
    pub fn lost(&self) -> u64 { self.lost }
    /// Stop: Bell drops the stream once it has played what is queued.
    pub fn close(&mut self) { self.ring.store(ring::OPEN, 0); }
}

impl Drop for Sound {
    fn drop(&mut self) {
        if self.listening { self.listen(false); }
        self.close();
    }
}
