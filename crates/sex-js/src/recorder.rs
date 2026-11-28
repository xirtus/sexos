// SPDX-License-Identifier: GPL-3.0-only
//! `MediaRecorder` for pages: what a page asked of its recorders, and the recorded
//! pieces waiting to be taken as Blobs. The browser encodes (sex-media's live
//! recorder), never the page's script. Sources are named by the page's tracks:
//! `camera`, `mic`, `canvas:N` (`canvas.captureStream`), and `audio:N` for a Web Audio
//! `MediaStreamAudioDestinationNode`'s track; a `-` before a name means the track is
//! disabled (black pictures, silence).
use alloc::string::String;
use alloc::vec::Vec;

/// Codecs the browser can record (bits of [`Recorders::codecs`]).
pub const VP8: u8 = 1;
pub const VP9: u8 = 2;
pub const OPUS: u8 = 4;

/// Most recorders a page may have at once.
pub const MAX_RECORDERS: usize = 16;
/// Most recorded bytes waiting for the page to take.
pub const MAX_WAITING: usize = 64 << 20;

/// What a type asks for: the picture codec (0: none) and whether sound may be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub video: u8,
    pub audio: bool,
    /// `audio/webm`: no pictures, whatever the stream holds.
    pub audio_only: bool,
    /// The type named its codecs.
    pub named: bool,
}

/// Reads a `MediaRecorder` type (`video/webm;codecs=vp8,opus`). None when this browser
/// cannot record it with `codecs`.
pub fn kind(codecs: u8, mime: &str) -> Option<Kind> {
    let mut parts = mime.split(';');
    let base = parts.next()?.trim().to_ascii_lowercase();
    let audio_only = match base.as_str() { "video/webm" => false, "audio/webm" => true, _ => return None };
    let mut list: Option<String> = None;
    for p in parts {
        let Some((k, v)) = p.split_once('=') else { if p.trim().is_empty() { continue } else { return None } };
        if k.trim().eq_ignore_ascii_case("codecs") { list = Some(String::from(v.trim().trim_matches('"'))); }
    }
    let mut k = Kind { video: 0, audio: true, audio_only, named: list.is_some() };
    match list {
        None => if !audio_only { k.video = if codecs & VP8 != 0 { VP8 } else if codecs & VP9 != 0 { VP9 } else { return None } },
        Some(list) => {
            k.audio = false;
            for c in list.split(',').map(|c| c.trim().to_ascii_lowercase()) {
                let v = if c == "vp8" || c.starts_with("vp8.") { VP8 }
                    else if c == "vp9" || c.starts_with("vp9.") || c.starts_with("vp09.") { VP9 }
                    else if c == "opus" { k.audio = true; continue }
                    else { return None };
                if audio_only || k.video != 0 || codecs & v == 0 { return None; }
                k.video = v;
            }
        }
    }
    if k.audio && codecs & OPUS == 0 { return None; }
    Some(k)
}

/// The type a recorder of `mime` (empty: the browser chooses) writes for a stream
/// with pictures and/or sound; None when it cannot. Gives the picture codec too.
pub fn choose(codecs: u8, mime: &str, video: bool, audio: bool) -> Option<(String, u8, bool)> {
    let chosen = mime.trim().is_empty();
    let k = kind(codecs, if !chosen { mime } else if video { "video/webm" } else { "audio/webm" })?;
    // video/webm;codecs=opus records only the sound of a stream with pictures.
    let v = if video { k.video } else { 0 };
    let a = audio && k.audio;
    if v == 0 && !a { return None; }
    let name = match v { VP8 => "vp8", VP9 => "vp9", _ => "" };
    let text = match (v != 0, a) {
        (true, true) => alloc::format!("video/webm;codecs={name},opus"),
        (true, false) => alloc::format!("video/webm;codecs={name}"),
        _ => String::from(if k.audio_only || chosen { "audio/webm;codecs=opus" } else { "video/webm;codecs=opus" }),
    };
    Some((text, v, a))
}

/// A page's call on one of its recorders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Start {
        id: u32,
        /// The picture source (`camera`, `canvas:N`, ...; empty: none) and sound source.
        video: String,
        audio: String,
        /// What it writes ([`choose`]) and its picture codec ([`VP8`], [`VP9`] or 0).
        mime: String,
        codec: u8,
        video_bps: u32,
        audio_bps: u32,
        /// Milliseconds per piece (0: one piece, at the end or on request).
        timeslice_ms: u32,
        fps: u32,
    },
    /// The track sources changed (a track was disabled, enabled or ended).
    Sources { id: u32, video: String, audio: String },
    Pause(u32),
    Resume(u32),
    /// `requestData`: hand over what is recorded so far.
    Data(u32),
    Stop(u32),
}

impl Ask {
    pub fn id(&self) -> u32 {
        match self { Ask::Start { id, .. } | Ask::Sources { id, .. } => *id, Ask::Pause(i) | Ask::Resume(i) | Ask::Data(i) | Ask::Stop(i) => *i }
    }
}

#[derive(Default)]
pub struct Recorders {
    /// What the browser can record (set by the browser; 0: nothing).
    pub codecs: u8,
    /// Calls not yet taken by the browser.
    pub asks: Vec<Ask>,
    /// Recorded pieces for the page, oldest first: (recorder, bytes).
    pub pieces: Vec<(u32, Vec<u8>)>,
    /// Recorders started and not yet stopped.
    pub running: Vec<u32>,
}

impl Recorders {
    /// A piece for the page; false when too much is waiting already (the page is not
    /// taking them), and the recorder should fail.
    pub fn give(&mut self, id: u32, bytes: Vec<u8>) -> bool {
        let waiting: usize = self.pieces.iter().map(|(_, b)| b.len()).sum();
        if waiting + bytes.len() > MAX_WAITING { return false; }
        self.pieces.push((id, bytes));
        true
    }

    /// The browser stopped recorder `id` itself (it failed, or its tracks ended).
    pub fn ended(&mut self, id: u32) { self.running.retain(|&r| r != id); }

    /// The oldest piece of recorder `id`.
    pub fn take(&mut self, id: u32) -> Option<Vec<u8>> {
        let i = self.pieces.iter().position(|(n, _)| *n == id)?;
        Some(self.pieces.remove(i).1)
    }

    /// Op RECORDER's text verbs (see [`crate::op::RECORDER`]).
    pub fn call(&mut self, a: &[&str]) -> Option<String> {
        let num = |i: usize| a.get(i).and_then(|v| v.trim().parse::<u32>().ok());
        let text = |i: usize| String::from(a.get(i).copied().unwrap_or("").trim());
        match *a.first()? {
            "type" => kind(self.codecs, a.get(1)?).map(|_| String::from("1")),
            "start" => {
                let id = num(1)?;
                if self.running.len() >= MAX_RECORDERS || self.running.contains(&id) { return None; }
                let (video, audio) = (text(2), text(3));
                let (mime, codec, keep_audio) = choose(self.codecs, a.get(4).copied().unwrap_or(""), !video.is_empty(), !audio.is_empty())?;
                let video = if codec == 0 { String::new() } else { video };
                let audio = if keep_audio { audio } else { String::new() };
                let video_bps = num(5).unwrap_or(2_500_000).clamp(100_000, 20_000_000);
                let audio_bps = num(6).unwrap_or(128_000).clamp(6_000, 510_000);
                let timeslice_ms = num(7).unwrap_or(0).min(3_600_000);
                let fps = num(8).filter(|&f| f > 0).unwrap_or(30).min(60);
                self.running.push(id);
                self.asks.push(Ask::Start { id, video, audio, mime: mime.clone(), codec, video_bps, audio_bps, timeslice_ms, fps });
                Some(mime)
            }
            verb => {
                let id = num(1)?;
                if !self.running.contains(&id) || self.asks.len() >= 256 { return None; }
                self.asks.push(match verb {
                    "sources" => Ask::Sources { id, video: text(2), audio: text(3) },
                    "pause" => Ask::Pause(id),
                    "resume" => Ask::Resume(id),
                    "data" => Ask::Data(id),
                    "stop" => { self.running.retain(|&r| r != id); Ask::Stop(id) }
                    _ => return None,
                });
                None
            }
        }
    }
}
