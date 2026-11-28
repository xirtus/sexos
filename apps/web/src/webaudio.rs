// SPDX-License-Identifier: GPL-3.0-only
//! Web Audio for pages: the graphs pages build (sex-js `op::AUDIO`) run here, in
//! `sex_webaudio`, a render quantum at a time, and what reaches their destinations
//! plays through the program's sound ring.
//!
//! A program has one ring. A playing `<video>`/`<audio>` keeps it (Web Audio is
//! silent meanwhile and its clock runs on ticks); once none plays, Web Audio takes
//! the ring when it has something to say and gives it back after two quiet seconds
//! or when an element starts playing (`release`, from `hush_media`).
//!
//! The microphone reaches the graphs through tap 0, which capture.rs fills; a
//! `MediaStreamAudioDestinationNode` writes a tap of its own (`Hub::read_tap`).
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_app::sound::Sound;
use sex_webaudio::{Decoded, Hub, QUANTUM};

/// Sound queued ahead of the card: 120 ms.
const LEAD: u64 = 5760;
/// Without the ring, time owed is capped at this (a stalled browser skips ahead).
const CREDIT_MAX: u64 = 4800;
/// Quiet quanta before the ring goes back: two seconds.
const QUIET_MAX: u32 = 750;

pub struct WebAudio {
    pub hub: Rc<RefCell<Hub>>,
    sound: Option<Sound>,
    last: u64,
    credit: u64,
    quiet: u32,
    /// Frames rendered, and of those heard (not silent), since the start.
    rendered: u64,
    heard: u64,
    peak: i32,
    logged: u64,
    playing: bool,
}

impl WebAudio {
    pub fn new() -> WebAudio {
        let mut hub = Hub::new();
        hub.decoder = Some(decode);
        WebAudio { hub: Rc::new(RefCell::new(hub)), sound: None, last: 0, credit: 0, quiet: 0, rendered: 0, heard: 0, peak: 0, logged: 0, playing: false }
    }

    /// A media element is about to take the program's ring.
    pub fn release(&mut self) {
        if let Some(s) = self.sound.take() {
            // The ring is the element's now: closing it would close theirs.
            core::mem::forget(s);
            sex_pdx::serial_record!("[web.audio.ring] held=0 why=media");
        }
    }

    /// Takes the ring (the browser hushed its media elements first).
    pub fn take_ring(&mut self) {
        if self.sound.is_some() { return; }
        self.sound = Sound::open();
        self.quiet = 0;
        sex_pdx::serial_record!("[web.audio.ring] held={}", self.sound.is_some() as u8);
    }

    /// Renders what is due. `media` is whether an element plays sound now. Returns
    /// when to come back, and whether Web Audio wants the ring (the browser hushes its
    /// elements and calls `take_ring`).
    pub fn tick(&mut self, now: u64, media: bool) -> (Option<u64>, bool) {
        let mut hub = self.hub.borrow_mut();
        hub.prune();
        if !hub.running() {
            self.last = now;
            self.credit = 0;
            if self.playing { self.playing = false; sex_pdx::serial_record!("[web.audio.stopped] frames={} heard={}", self.rendered, self.heard); }
            if let Some(mut s) = self.sound.take() { s.close(); }
            return (None, false);
        }
        if media { drop(hub); self.release(); hub = self.hub.borrow_mut(); }
        let elapsed = now.saturating_sub(self.last).min(1000);
        if !self.playing { self.playing = true; self.credit = LEAD / 2; self.logged = now; sex_pdx::serial_record!("[web.audio.running] contexts={}", hub.contexts.len()); }
        self.last = now;
        let mut due = match self.sound.as_ref() {
            Some(s) => LEAD.saturating_sub(s.pending()),
            None => { self.credit = (self.credit + elapsed * 48).min(CREDIT_MAX); self.credit }
        };
        let mut want = false;
        let mut q = [[0i32; 2]; QUANTUM];
        let mut frames: Vec<[i16; 2]> = Vec::new();
        while due >= QUANTUM as u64 {
            due -= QUANTUM as u64;
            if self.sound.is_none() { self.credit -= QUANTUM as u64; }
            let heard = hub.render(&mut q);
            self.rendered += QUANTUM as u64;
            if heard {
                self.heard += QUANTUM as u64;
                self.quiet = 0;
                for f in &q { self.peak = self.peak.max(f[0].abs()).max(f[1].abs()); }
                if self.sound.is_none() && !media { want = true; }
            } else {
                self.quiet = self.quiet.saturating_add(1);
            }
            if self.sound.is_some() { frames.extend(q.iter().map(|f| [sex_webaudio::clip(f[0]), sex_webaudio::clip(f[1])])); }
        }
        drop(hub);
        if let Some(s) = self.sound.as_mut() {
            if !frames.is_empty() { s.write(&frames); }
            if self.quiet > QUIET_MAX { if let Some(mut s) = self.sound.take() { s.close(); } sex_pdx::serial_record!("[web.audio.ring] held=0 why=quiet"); }
        }
        if now >= self.logged + 1000 {
            self.logged = now;
            sex_pdx::serial_record!("[web.audio.out] frames={} heard={} peak={} ring={}", self.rendered, self.heard, self.peak, self.sound.is_some() as u8);
            self.peak = 0;
        }
        (Some(20), want)
    }

    /// Dispatches for page `owner` (sources that ended).
    pub fn events(&mut self, owner: u64) -> Vec<alloc::string::String> {
        let mut hub = self.hub.borrow_mut();
        if !hub.events.iter().any(|(o, _)| *o == owner) { return Vec::new(); }
        let mut out = Vec::new();
        hub.events.retain(|(o, e)| if *o == owner { out.push(e.clone()); false } else { true });
        out
    }
}

/// `decodeAudioData` through sex-media's decoders: 48 kHz stereo.
fn decode(bytes: &[u8]) -> Option<Decoded> {
    let mut dec = sex_media::Decoder::open(Box::new(bytes.to_vec())).ok()?;
    if dec.info.sample_rate == 0 { return None; }
    let channels = dec.info.channels.max(1);
    let mut pix = [0u32; 1];
    let mut pcm = alloc::vec![[0i16; 2]; 4096];
    let mut frames = Vec::new();
    loop {
        match dec.next(&mut pix, 1, 1, &mut pcm) {
            Ok(sex_media::Next::Sound { frames: n, .. }) => frames.extend_from_slice(&pcm[..n.min(pcm.len())]),
            Ok(sex_media::Next::Picture { .. }) => {}
            _ => break,
        }
        // Ten minutes at most.
        if frames.len() > 48_000 * 600 { break; }
    }
    sex_pdx::serial_record!("[web.audio.decode] bytes={} frames={} channels={} codec={}", bytes.len(), frames.len(), channels, dec.info.audio_codec);
    if frames.is_empty() { return None; }
    Some(Decoded { rate: sex_webaudio::RATE, channels, frames })
}
