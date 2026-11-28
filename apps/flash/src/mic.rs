// SPDX-License-Identifier: GPL-3.0-only
//! Sound for Flash's videos: while a video records, Flash listens to Bell's
//! microphone (whichever input Bell has chosen: a headset, a USB mic or the
//! built-in mics) and hands each heard block to the exporter beside the pictures.
//!
//! The microphone is the clock while it is on. Until it is on (the person has not
//! allowed it yet, or there is no microphone), Flash writes silence for the time the
//! pictures cover, so sound and pictures stay in step and the file always has sound.
use sex_app::sound::Sound;
use sex_media::export::Exporter;
use sex_pdx::serial_record as log;
use sex_pdx::sound_ring::{MIC_NONE, MIC_ON};

/// Bell's microphone rate (stereo i16 frames per second).
const RATE: u64 = 48_000;
/// Frames read from the ring per call to the exporter.
const BLOCK: usize = 1024;

pub struct Listen {
    sound: Option<Sound>,
    /// Sound frames given to the exporter since the video started.
    written: u64,
    heard: u64,
    silence: u64,
    told: u64,
}

impl Listen {
    /// Starts listening. Without Bell's ring the video still gets silence.
    pub fn start() -> Listen {
        let mut sound = Sound::open();
        if let Some(s) = sound.as_mut() { s.listen(true); }
        log!("[flash.mic.start] ring={}", sound.is_some() as u8);
        Listen { sound, written: 0, heard: 0, silence: 0, told: u64::MAX }
    }

    /// Gives the exporter the sound up to the end of picture `pictures` (a count) at
    /// `fps` pictures a second.
    pub fn feed(&mut self, exp: &mut Exporter, pictures: u64, fps: u32, drain: &mut dyn FnMut(&mut Exporter)) {
        let state = self.sound.as_ref().map_or(MIC_NONE, |s| s.mic());
        if state != self.told {
            log!("[flash.mic.state] mic={}", state);
            self.told = state;
        }
        let mut block = [[0i16; 2]; BLOCK];
        if let Some(s) = self.sound.as_mut() {
            loop {
                let n = s.hear(&mut block);
                if n == 0 { break; }
                if state == MIC_ON {
                    if exp.sound(&block[..n]).is_ok() { self.written += n as u64; self.heard += n as u64; }
                    drain(exp);
                }
                if n < BLOCK { break; }
            }
        }
        if state == MIC_ON { return; }
        let target = pictures * RATE / fps.max(1) as u64;
        block = [[0; 2]; BLOCK];
        while self.written < target {
            let n = ((target - self.written) as usize).min(BLOCK);
            if exp.sound(&block[..n]).is_err() { break; }
            drain(exp);
            self.written += n as u64;
            self.silence += n as u64;
        }
    }

    /// Stops listening and reports what the video got.
    pub fn stop(mut self) {
        if let Some(s) = self.sound.as_mut() {
            s.listen(false);
            log!("[flash.mic.stop] heard={} silence={} lost={}", self.heard, self.silence, s.lost());
            s.close();
        } else {
            log!("[flash.mic.stop] heard=0 silence={} lost=0", self.silence);
        }
    }
}
