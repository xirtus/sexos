// SPDX-License-Identifier: GPL-3.0-only
//! `MediaRecorder` for pages: the browser records, never the page's script. Each
//! recorder takes its pictures from the camera's latest picture or a canvas's (a
//! `canvas.captureStream`), and its sound from the microphone (or, through
//! [`Recorders::feed_audio`], from a Web Audio `MediaStreamAudioDestinationNode`),
//! and encodes them with sex-media's live recorder into WebM (VP8 or VP9, Opus).
//! Pieces go to the page as Blobs (sex-js `op::RECORDER`).
//!
//! Recording keeps the page responsive: at most one picture per recorder per pass of
//! the browser's loop, and a pacer that spaces pictures out when encoding is slow
//! (QEMU without KVM, a large canvas). Sound always keeps up: a quiet or blocked
//! microphone records silence so the sound lasts as long as the pictures.
use alloc::string::String;
use alloc::vec::Vec;
use sex_js::recorder::{Ask, OPUS, VP8, VP9};
use sex_media::live::{self, Live, Pacer, Video};
use sex_web::Fonts;

/// The biggest recording: what a video element can show.
const MAX_W: usize = crate::media::MAX_W;
const MAX_H: usize = crate::media::MAX_H;
/// How long a recorder waits for its first picture before it opens at 640x480.
const FIRST_PICTURE_MS: u64 = 1000;
/// A canvas that does not change still gives a picture this often.
const STILL_MS: u64 = 1000;
/// Sound more than this far behind the pictures is filled with silence.
const SOUND_BEHIND_MS: u64 = 200;
/// Microphone sound more than this far ahead of the pictures is left out.
const SOUND_AHEAD_MS: u64 = 500;

/// What the browser can record, for sex-js (`Recorders::codecs`).
pub fn codecs() -> u8 {
    let mut c = 0;
    if live::supported(Some(Video::Vp8), false) { c |= VP8; }
    if live::supported(Some(Video::Vp9), false) { c |= VP9; }
    if live::supported(None, true) { c |= OPUS; }
    c
}

/// What the browser tells a page, in order.
pub enum Out {
    /// A piece of recorder `id`'s file for the page to take, then `recorder\tID\tdata\tN`.
    Piece(u64, u32, Vec<u8>),
    /// A dispatch for the page of `owner`.
    Event(u64, String),
}

/// Where a recorder's pictures and sound come from now (what the browser has).
pub struct Sources<'a> {
    pub owner: u64,
    /// The camera's latest picture and its number, when `owner` holds the camera.
    pub camera: Option<(&'a sex_web::Image, u32)>,
    /// What the microphone gave since the last pass, when `owner` holds it.
    pub mic: Option<&'a [[i16; 2]]>,
    pub fonts: &'a Fonts,
}

struct Rec {
    owner: u64,
    id: u32,
    video: String,
    audio: String,
    mime: String,
    codec: u8,
    video_bps: u32,
    audio_bps: u32,
    timeslice: u64,
    fps: u32,
    live: Option<Live>,
    begun: u64,
    paused_at: Option<u64>,
    paused_ms: u64,
    last_piece: u64,
    pacer: Pacer,
    seen_seq: u32,
    /// The canvas drew since its last picture.
    drawn: bool,
    last_frame: Option<u64>,
    black: Vec<u32>,
    /// Sound from a Web Audio node, not yet recorded.
    fed: Vec<[i16; 2]>,
    cost_ms: u64,
    bytes: u64,
    /// The page called `stop`.
    stopping: bool,
    /// Told the page it stopped: goes on the next pass.
    done: bool,
}

#[derive(Default)]
pub struct Recorders {
    list: Vec<Rec>,
}

impl Recorders {
    pub fn active(&self) -> bool { !self.list.is_empty() }

    /// Canvases that drew (sex-web's `Canvases::drew`): their recorders take a picture.
    pub fn drawn(&mut self, nodes: &[usize]) {
        for r in self.list.iter_mut() {
            if let Some(n) = canvas_of(&r.video) { if nodes.contains(&n) { r.drawn = true; } }
        }
    }

    /// Sound for the recorders of `owner` whose sound source is `source` (`audio:N`, a
    /// Web Audio MediaStreamAudioDestinationNode's track): the hook Web Audio feeds
    /// with what that node played, 48 kHz stereo.
    #[allow(dead_code)] // Web Audio (C2) feeds it
    pub fn feed_audio(&mut self, owner: u64, source: &str, pcm: &[[i16; 2]]) {
        for r in self.list.iter_mut().filter(|r| r.owner == owner && r.audio == source && r.paused_at.is_none()) {
            if r.fed.len() + pcm.len() <= live::RATE as usize * 2 { r.fed.extend_from_slice(pcm); }
        }
    }

    /// A page went away: its recorders end without telling it.
    pub fn page_left(&mut self, owner: u64) {
        self.list.retain(|r| {
            if r.owner == owner { sex_pdx::serial_record!("[web.recorder.dropped] id={}", r.id); }
            r.owner != owner
        });
    }

    /// The page's calls on its recorders.
    pub fn take(&mut self, owner: u64, asks: Vec<Ask>, now: u64, out: &mut Vec<Out>) {
        for ask in asks {
            let id = ask.id();
            let Some(i) = self.list.iter().position(|r| r.owner == owner && r.id == id).or_else(|| {
                let Ask::Start { video, audio, mime, codec, video_bps, audio_bps, timeslice_ms, fps, .. } = &ask else { return None };
                sex_pdx::serial_record!("[web.recorder.ask] id={} video={} audio={} mime={} slice={} fps={}", id, video, audio, mime, timeslice_ms, fps);
                self.list.push(Rec {
                    owner, id, video: video.clone(), audio: audio.clone(), mime: mime.clone(), codec: *codec, video_bps: *video_bps, audio_bps: *audio_bps,
                    timeslice: *timeslice_ms as u64, fps: (*fps).clamp(1, 30), live: None, begun: now, paused_at: None, paused_ms: 0, last_piece: 0,
                    pacer: Pacer::new((*fps).clamp(1, 30)), seen_seq: 0, drawn: true, last_frame: None, black: Vec::new(), fed: Vec::new(), cost_ms: 0, bytes: 0, stopping: false, done: false,
                });
                Some(self.list.len() - 1)
            }) else { continue };
            let r = &mut self.list[i];
            match ask {
                Ask::Start { .. } => {}
                Ask::Sources { video, audio, .. } => { r.video = video; r.audio = audio; }
                Ask::Pause(_) => { if r.paused_at.is_none() { r.paused_at = Some(now); } }
                Ask::Resume(_) => { if let Some(p) = r.paused_at.take() { r.paused_ms += now.saturating_sub(p); } }
                Ask::Data(_) => r.piece(out),
                Ask::Stop(_) => r.stopping = true,
            }
        }
    }

    /// Records what is due for every recorder; gives what the pages are told. Returns
    /// how soon to come back.
    pub fn tick(&mut self, now: u64, src: &Sources, out: &mut Vec<Out>) -> Option<u64> {
        for r in self.list.iter_mut() {
            if r.done || (r.owner != src.owner && !r.stopping) { continue; }
            if r.live.is_none() {
                if r.stopping { r.finish(now, out); continue; }
                if !r.open(now, src, out) { continue; }
            }
            if r.paused_at.is_none() && !r.stopping { r.record(now, src); }
            if r.stopping { r.finish(now, out); continue; }
            if r.live.as_ref().is_some_and(|l| l.error().is_some()) { r.fail(out); continue; }
            if r.timeslice > 0 && r.paused_at.is_none() && r.time(now) >= r.last_piece + r.timeslice {
                r.last_piece = r.time(now);
                r.piece(out);
            }
        }
        self.list.retain(|r| !r.done);
        self.active().then_some(10)
    }
}

fn canvas_of(src: &str) -> Option<usize> { src.trim_start_matches('-').strip_prefix("canvas:")?.parse().ok() }

impl Rec {
    /// Milliseconds recorded (paused time left out).
    fn time(&self, now: u64) -> u64 {
        let paused = self.paused_ms + self.paused_at.map_or(0, |p| now.saturating_sub(p));
        now.saturating_sub(self.begun).saturating_sub(paused)
    }

    /// The picture the source shows now: pixels, width, height, and whether it is new.
    fn picture<'a>(&mut self, src: &'a Sources) -> Option<(&'a [u32], usize, usize, bool)> {
        let video = self.video.trim_start_matches('-');
        if video == "camera" {
            let (img, seq) = src.camera?;
            let new = seq != self.seen_seq;
            self.seen_seq = seq;
            return Some((&img.pixels, img.width, img.height, new));
        }
        let name = alloc::format!("canvas:{}", canvas_of(video)?);
        let img = src.fonts.images.iter().find(|(s, _)| *s == name).map(|(_, i)| i)?;
        Some((&img.pixels, img.width, img.height, core::mem::take(&mut self.drawn)))
    }

    /// Opens the encoder once the first picture tells its size. False while waiting.
    fn open(&mut self, now: u64, src: &Sources, out: &mut Vec<Out>) -> bool {
        let video = if self.video.is_empty() { None } else if self.codec == VP9 { Some(Video::Vp9) } else { Some(Video::Vp8) };
        let (mut w, mut h) = (0, 0);
        if video.is_some() {
            match self.picture(src) {
                Some((_, pw, ph, _)) => { (w, h) = (pw, ph); self.drawn = true; self.seen_seq = 0; }
                None if now < self.begun + FIRST_PICTURE_MS => return false,
                None => {}
            }
        }
        let (w, h) = live::size_for(w, h, MAX_W, MAX_H);
        let config = live::Config { video, width: w, height: h, fps: self.fps, video_bps: self.video_bps, audio: !self.audio.is_empty(), audio_bps: self.audio_bps };
        match Live::new(config) {
            Ok(l) => {
                sex_pdx::serial_record!("[web.recorder.start] id={} mime={} w={} h={} fps={} video={} audio={}", self.id, self.mime, w, h, self.fps, self.video, self.audio);
                // The file's start (its header) waits there for the first piece.
                self.live = Some(l);
                // Time starts now, once there is something to record.
                self.begun = now;
                out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\tstart", self.id)));
                true
            }
            Err(e) => {
                sex_pdx::serial_record!("[web.recorder.failed] id={} err={:?}", self.id, e);
                out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\terror\tNotSupportedError", self.id)));
                out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\tstop", self.id)));
                self.done = true;
                false
            }
        }
    }

    /// One pass: a picture if one is due, and the sound since the last pass.
    fn record(&mut self, now: u64, src: &Sources) {
        let t = self.time(now);
        let Some(mut live) = self.live.take() else { return };
        if live.config().video.is_some() {
            let disabled = self.video.starts_with('-') || self.video.is_empty();
            let at = t.max(live.last_picture_ms().map_or(0, |l| l + 1)).max(1);
            let still = self.last_frame.is_none_or(|l| t >= l + STILL_MS);
            let picture = if disabled { None } else { self.picture(src) };
            let fresh = picture.as_ref().is_some_and(|p| p.3) || still || disabled;
            if fresh && self.pacer.due(at) {
                let begun = sex_pdx::get_ticks();
                let r = match picture {
                    Some((px, w, h, _)) => live.picture(px, w, h, at),
                    None => {
                        // A disabled track, or a source that went: black pictures.
                        let c = live.config();
                        let n = c.width as usize * c.height as usize;
                        if self.black.len() != n { self.black = alloc::vec![0; n]; }
                        live.picture(&self.black, c.width as usize, c.height as usize, at)
                    }
                };
                let spent = sex_pdx::get_ticks().saturating_sub(begun);
                self.cost_ms += spent;
                self.pacer.spent(at, spent);
                self.last_frame = Some(t);
                if r.is_err() { sex_pdx::serial_record!("[web.recorder.picture.failed] id={} err={:?}", self.id, r); }
                if live.pictures <= 3 || live.pictures.is_power_of_two() {
                    sex_pdx::serial_record!("[web.recorder.picture] id={} n={} at={} ms={} interval={}", self.id, live.pictures, at, spent, self.pacer.interval_ms());
                }
            }
        }
        if live.config().audio {
            let due = t * live::RATE as u64 / 1000;
            let ahead = live.samples > due + SOUND_AHEAD_MS * 48;
            let pcm: &[[i16; 2]] = match self.audio.as_str() {
                "mic" => src.mic.unwrap_or(&[]),
                s if s.starts_with("audio:") => &self.fed,
                _ => &[],
            };
            if !ahead && !pcm.is_empty() { let _ = live.sound(pcm); }
            self.fed.clear();
            if live.samples + SOUND_BEHIND_MS * 48 < due { let _ = live.sound_until(t.saturating_sub(SOUND_BEHIND_MS / 2)); }
        }
        self.live = Some(live);
    }

    /// What is recorded so far goes to the page as a piece.
    fn piece(&mut self, out: &mut Vec<Out>) {
        let Some(l) = self.live.as_mut() else {
            out.push(Out::Piece(self.owner, self.id, Vec::new()));
            return;
        };
        let _ = l.flush();
        let bytes = l.take();
        self.bytes += bytes.len() as u64;
        sex_pdx::serial_record!("[web.recorder.piece] id={} bytes={} pictures={} samples={}", self.id, bytes.len(), l.pictures, l.samples);
        out.push(Out::Piece(self.owner, self.id, bytes));
    }

    fn finish(&mut self, now: u64, out: &mut Vec<Out>) {
        let t = self.time(now);
        if let Some(l) = self.live.as_mut() {
            if l.config().audio { let _ = l.sound_until(t); }
            if let Err(e) = l.finish() { sex_pdx::serial_record!("[web.recorder.finish.failed] id={} err={:?}", self.id, e); }
            let bytes = l.take();
            self.bytes += bytes.len() as u64;
            sex_pdx::serial_record!("[web.recorder.stop] id={} ms={} pictures={} skipped={} samples={} bytes={} encode_ms={}",
                self.id, t, l.pictures, self.pacer.skipped, l.samples, self.bytes, self.cost_ms);
            out.push(Out::Piece(self.owner, self.id, bytes));
        } else {
            out.push(Out::Piece(self.owner, self.id, Vec::new()));
        }
        out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\tstop", self.id)));
        self.live = None;
        self.done = true;
    }

    fn fail(&mut self, out: &mut Vec<Out>) {
        sex_pdx::serial_record!("[web.recorder.failed] id={} err={:?}", self.id, self.live.as_ref().and_then(|l| l.error()));
        out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\terror\tUnknownError", self.id)));
        out.push(Out::Event(self.owner, alloc::format!("recorder\t{}\tstop", self.id)));
        self.live = None;
        self.done = true;
    }
}
