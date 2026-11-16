// SPDX-License-Identifier: GPL-3.0-only
//! Headphones (W3): Bell's mix, from the `sex_pdx::bt_audio` region, to the headset's
//! A2DP stream.
//!
//! - When a stream is ready (`Note::AudioReady`) Cufflinks turns the region's sink on,
//!   and Bell sends its mix there instead of to the speakers; when the stream or the
//!   link goes, the sink goes off and the speakers come back.
//! - Pacing: the headset plays at its own 48 kHz and has a small buffer, while Bell
//!   renders ahead of its card. Cufflinks sends no more than real time allows by its
//!   own clock (the kernel's 1 ms tick) plus a short lead, and drops the oldest frames
//!   when Bell is more than `MAX_BACKLOG` ahead, so latency stays bounded.
//! - With nothing to play for `IDLE_SUSPEND` the stream is suspended (the headset may
//!   sleep its radio); sound in the region starts it again.
//! - A short chime plays when a headset's stream first starts: the receipt that the
//!   whole path (SBC, AVDTP, the radio) works, even before any program plays sound.

use alloc::vec::Vec;
use sex_bt::avdtp::State;
use sex_bt::host::Host;
use sex_pdx::bt_audio::{self, Shared};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::serial_record as log;

/// Frames sent ahead of real time when a stream (re)starts: 100 ms.
const LEAD: u64 = 4_800;
/// Frames per millisecond.
const PER_MS: u64 = bt_audio::RATE as u64 / 1000;
/// Bell may be this far ahead of the headset before the oldest frames are dropped.
const MAX_BACKLOG: u32 = 36_000;
/// Silence long enough to suspend the stream, in ms.
const IDLE_SUSPEND: u64 = 3_000;
/// Frames looked at per pass.
const CHUNK: usize = 2_048;

pub struct Audio {
    ring: Option<Shared>,
    handle: Option<u16>,
    /// Pacing: when this run of sound started, and frames sent since.
    t0: u64,
    sent: u64,
    last_sound: u64,
    chime: Vec<[i16; 2]>,
    chimed: bool,
    buf: Vec<[i16; 2]>,
    pub total: u64,
    pub dropped: u64,
}

/// Bhaskara's sine for a phase in degrees (0..360), times 32767.
fn sine(deg: u32) -> i32 {
    let (x, neg) = if deg >= 180 { (deg - 180, true) } else { (deg, false) };
    let p = (x * (180 - x)) as i64;
    let v = (4 * p * 32767 / (40_500 - p)) as i32;
    if neg { -v } else { v }
}

/// Two short rising notes, quiet.
fn chime() -> Vec<[i16; 2]> {
    let mut out = Vec::with_capacity(48 * 360);
    for (hz, ms) in [(880u32, 150u32), (1320, 210)] {
        let n = 48 * ms;
        for i in 0..n {
            let deg = ((i as u64 * hz as u64 * 360 / 48_000) % 360) as u32;
            // 5 ms ramps at both ends.
            let edge = i.min(n - 1 - i).min(240);
            let s = (sine(deg) as i64 * 6 * edge as i64 / (100 * 240)) as i16;
            out.push([s, s]);
        }
    }
    out
}

impl Audio {
    /// The region BOOT shares with Bell, checked (Bell's peer view, our length).
    fn region() -> Option<Shared> {
        let info = sex_pdx::memory_region_info(bt_audio::SLOT).ok()?;
        if info.region_id != bt_audio::REGION_ID || info.length != bt_audio::BYTES || info.peer_pd.is_none() { return None; }
        let v = sex_pdx::memory_region_full_view(bt_audio::SLOT, QUERY_VIEW | READ | WRITE).ok()?;
        (v.length >= bt_audio::BYTES && v.virtual_address != 0).then(|| unsafe { Shared::at(v.virtual_address as *mut u8) })
    }
    /// At start: log whether Bell's region is there.
    pub fn probe() { log!("[cufflinks.audio.route] ok={} slot={}", Self::region().is_some() as u8, bt_audio::SLOT); }

    pub fn new() -> Audio {
        let ring = Self::region();
        if let Some(r) = ring { r.set_sink(false); r.set_headset_volume(false); }
        Audio { ring, handle: None, t0: 0, sent: 0, last_sound: 0, chime: Vec::new(), chimed: false, buf: alloc::vec![[0; 2]; CHUNK], total: 0, dropped: 0 }
    }

    pub fn handle(&self) -> Option<u16> { self.handle }
    /// Bell's master volume (0..=100), once Bell has written it.
    pub fn master(&self) -> Option<u8> { self.ring?.master().map(|m| m.0) }
    /// Tell Bell the headset applies the master volume itself (AVRCP absolute volume).
    pub fn headset_volume(&self, on: bool) {
        if let Some(r) = self.ring { r.set_headset_volume(on); }
        log!("[cufflinks.audio.volume] headset={}", on as u8);
    }

    /// A stream is ready on `handle` (the first one wins while it lasts).
    pub fn ready(&mut self, handle: u16) {
        if self.handle.is_some_and(|h| h != handle) { return; }
        self.handle = Some(handle);
        self.t0 = 0;
        self.chimed = false;
        if let Some(r) = self.ring { r.set_sink(true); }
    }
    /// It streams: the first time, queue the chime.
    pub fn streaming(&mut self, handle: u16) {
        if self.handle == Some(handle) && !self.chimed { self.chimed = true; self.chime = chime(); }
    }
    /// The stream or its link is gone.
    pub fn gone(&mut self, handle: u16) {
        if self.handle != Some(handle) { return; }
        self.handle = None;
        self.chime.clear();
        if let Some(r) = self.ring { r.set_sink(false); }
        log!("[cufflinks.audio.stopped] handle={:#06x} frames={} dropped={}", handle, self.total, self.dropped);
    }

    /// Send what the pace allows. True when something moved.
    pub fn pump(&mut self, h: &mut Host, now: u64) -> bool {
        let Some(handle) = self.handle else { return false };
        let ring = self.ring;
        let waiting = ring.map(|r| r.queued()).unwrap_or(0) as usize + self.chime.len();
        let state = h.audio_state(handle);
        if waiting < 128 {
            if state == Some(State::Streaming) && now.saturating_sub(self.last_sound) > IDLE_SUSPEND {
                log!("[cufflinks.audio.idle] handle={:#06x} suspend=1", handle);
                h.suspend_audio(handle);
                return true;
            }
            return false;
        }
        self.last_sound = now;
        match state {
            Some(State::Streaming) => {}
            Some(State::Open) => { h.resume_audio(handle); self.t0 = 0; return true; }
            _ => return false,
        }
        // Bell far ahead (its card's clock runs faster than the headset's): drop the oldest.
        if let Some(r) = ring {
            let q = r.queued();
            if q > MAX_BACKLOG { r.consume((q - MAX_BACKLOG / 2) as usize); self.dropped += (q - MAX_BACKLOG / 2) as u64; }
        }
        // Start the clock on a new or resumed stream; after an underrun, send at most the
        // lead again rather than everything the gap would allow.
        if self.t0 == 0 || self.t0 > now { self.t0 = now; self.sent = 0; }
        let allowed = (now - self.t0) * PER_MS + LEAD;
        if allowed > self.sent + 2 * LEAD { self.sent = allowed - LEAD; }
        if allowed <= self.sent { return false; }
        let want = ((allowed - self.sent) as usize).min(CHUNK);
        let mut moved = false;
        if !self.chime.is_empty() {
            let n = want.min(self.chime.len());
            let used = h.play(handle, &self.chime[..n]);
            self.chime.drain(..used);
            // A chime shorter than one SBC frame at its end: drop it.
            if self.chime.len() < 128 { self.chime.clear(); }
            self.sent += used as u64;
            return used > 0;
        }
        if let Some(r) = ring {
            let n = r.peek(&mut self.buf[..want]);
            let used = h.play(handle, &self.buf[..n]);
            r.consume(used);
            self.sent += used as u64;
            self.total += used as u64;
            moved = used > 0;
        }
        moved
    }
}
