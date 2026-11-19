// SPDX-License-Identifier: GPL-3.0-only
//! Text to speech for SexOS programs: eSpeak NG 1.52.0 (GPL-3.0-or-later,
//! `vendor/espeak-ng-1.52.0`) with its English data compiled in, so the voice needs no
//! file system and no authority beyond the program that links it. Whisper, the screen
//! reader, speaks through `apps/whisper-voice`, a confined program that links this.
//!
//! ```ignore
//! let spoken = sex_speech::speak("OK button", 175, &mut |pcm: &[i16]| play(pcm))?;
//! ```
//!
//! Output is mono signed 16-bit PCM at [`SAMPLE_RATE`] (22 050 Hz). Bell converts any
//! rate a program's ring names (`sound_ring::RATE_HZ`), so callers write it as is.
//! Synthesis is deterministic: the same text and rate give the same samples.
//!
//! [`stop`] ends the utterance being spoken at its next buffer (at most 100 ms of
//! sound later). It may be called from inside the sink or from another thread.
//!
//! eSpeak NG keeps global state, so one utterance is synthesised at a time; a second
//! caller waits for the first to finish.
#![cfg_attr(not(test), no_std)]

extern crate alloc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Samples a second of the PCM [`speak`] produces (eSpeak NG's own rate).
pub const SAMPLE_RATE: u32 = 22_050;
/// Slowest and fastest speaking rates, in words a minute (eSpeak NG's range).
pub const MIN_WPM: u16 = 80;
pub const MAX_WPM: u16 = 450;
/// eSpeak NG's normal rate.
pub const DEFAULT_WPM: u16 = 175;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// This build has no voice (no C compiler or musl headers when it was built).
    Unavailable,
    /// eSpeak NG refused to start or to speak; its status code.
    Engine(i32),
}

/// What one call to [`speak`] produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Spoken {
    /// Mono samples handed to the sink.
    pub samples: usize,
    /// True when [`stop`] cut the utterance short.
    pub stopped: bool,
}

impl Spoken {
    /// Length of the sound in milliseconds.
    pub fn ms(&self) -> u64 { self.samples as u64 * 1000 / SAMPLE_RATE as u64 }
}

static STOP: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);
/// eSpeak NG's sample rate once started; 0 before, negative when it failed.
static STARTED: AtomicI32 = AtomicI32::new(0);

/// Ends the utterance being spoken at its next buffer. Does nothing when none is.
pub fn stop() { if BUSY.load(Ordering::Acquire) { STOP.store(true, Ordering::Release) } }

/// True while an utterance is being synthesised.
pub fn speaking() -> bool { BUSY.load(Ordering::Acquire) }

struct Sink<'a> {
    out: &'a mut dyn FnMut(&[i16]),
    samples: usize,
    stopped: bool,
}

/// Speaks `text` (UTF-8, plain text) at `rate_wpm` words a minute (clamped to
/// [`MIN_WPM`]..=[`MAX_WPM`]), handing the sound to `out` in pieces of mono PCM at
/// [`SAMPLE_RATE`]. Returns when the utterance is finished or stopped. Empty or
/// blank text produces no sound.
pub fn speak(text: &str, rate_wpm: u16, out: &mut impl FnMut(&[i16])) -> Result<Spoken, Error> {
    let text = text.split('\0').next().unwrap_or("");
    if text.trim().is_empty() { return Ok(Spoken::default()) }
    while BUSY.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    STOP.store(false, Ordering::Release);
    let result = speak_locked(text, rate_wpm.clamp(MIN_WPM, MAX_WPM), out);
    STOP.store(false, Ordering::Release);
    BUSY.store(false, Ordering::Release);
    result
}

#[cfg(espeak)]
fn speak_locked(text: &str, wpm: u16, out: &mut dyn FnMut(&[i16])) -> Result<Spoken, Error> {
    extern "C" {
        fn sexos_speech_init() -> i32;
        fn sexos_speech_say(text: *const u8, len: usize, wpm: i32, sink: *mut core::ffi::c_void) -> i32;
    }
    let mut rate = STARTED.load(Ordering::Acquire);
    if rate == 0 {
        rate = unsafe { sexos_speech_init() };
        STARTED.store(if rate == 0 { -1 } else { rate }, Ordering::Release);
    }
    if rate <= 0 { return Err(Error::Engine(rate)) }
    let mut z = Vec::with_capacity(text.len() + 1);
    z.extend_from_slice(text.as_bytes());
    z.push(0);
    let mut sink = Sink { out, samples: 0, stopped: false };
    let status = unsafe { sexos_speech_say(z.as_ptr(), text.len(), wpm as i32, &mut sink as *mut Sink as *mut _) };
    if status != 0 { return Err(Error::Engine(status)) }
    Ok(Spoken { samples: sink.samples, stopped: sink.stopped })
}

#[cfg(not(espeak))]
fn speak_locked(_: &str, _: u16, _: &mut dyn FnMut(&[i16])) -> Result<Spoken, Error> {
    let _ = (&STARTED, Vec::<u8>::new());
    Err(Error::Unavailable)
}

/// Called by eSpeak NG (csrc/sexos_espeak_shim.c) with each buffer of sound.
#[cfg(espeak)]
#[no_mangle]
extern "C" fn sexos_speech_samples(sink: *mut core::ffi::c_void, wav: *const i16, n: i32) -> i32 {
    let sink = unsafe { &mut *(sink as *mut Sink) };
    if STOP.load(Ordering::Acquire) { sink.stopped = true; return 1 }
    if !wav.is_null() && n > 0 {
        let pcm = unsafe { core::slice::from_raw_parts(wav, n as usize) };
        sink.samples += pcm.len();
        (sink.out)(pcm);
    }
    // A stop asked for inside the sink takes effect now, not a buffer later.
    if STOP.load(Ordering::Acquire) { sink.stopped = true; return 1 }
    0
}

/// The data eSpeak NG reads, compiled in. Paths are relative to its data root.
static FILES: &[(&str, &[u8])] = &[
    ("phontab", include_bytes!("../espeak-ng-data/phontab")),
    ("phonindex", include_bytes!("../espeak-ng-data/phonindex")),
    ("phondata", include_bytes!("../espeak-ng-data/phondata")),
    ("intonations", include_bytes!("../espeak-ng-data/intonations")),
    ("en_dict", include_bytes!("../espeak-ng-data/en_dict")),
    ("lang/gmw/en", include_bytes!("../espeak-ng-data/lang/gmw/en")),
];

/// The embedded file `name` (NUL-terminated), for the C shim's memory streams.
#[cfg(espeak)]
#[no_mangle]
extern "C" fn sexos_speech_file(name: *const u8, len: *mut usize) -> *const u8 {
    let name = unsafe { core::ffi::CStr::from_ptr(name as *const core::ffi::c_char) }.to_bytes();
    match FILES.iter().find(|(n, _)| n.as_bytes() == name) {
        Some((_, data)) => { unsafe { *len = data.len() }; data.as_ptr() }
        None => core::ptr::null(),
    }
}

/// Bytes of voice data compiled into the program.
pub fn data_bytes() -> usize { FILES.iter().map(|(_, d)| d.len()).sum() }
