// SPDX-License-Identifier: GPL-3.0-only
//! `<video>` and `<audio>`: each element gets a sex-media player. Its pictures become
//! the page picture `video:N` (and the controls of a sound element `audio:N`), which
//! the browser repaints alone when nothing covers them. Sound goes to this program's
//! sound ring. Scripts drive players through `HTMLMediaElement` (sex-js), and hear
//! back through `media` dispatches.
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use sex_html::dom::{Dom, NodeId};
use sex_media::{Decoder, Player, Step};
use sex_web::sex_html;
use sex_web::{Fonts, Image};

/// Largest decoded picture: the window's content area, which is all a video can fill.
pub const MAX_W: usize = 640;
pub const MAX_H: usize = 480;
/// Height of the control bar drawn over a video with `controls`.
const BAR_H: usize = 20;
/// How often a playing element reports `timeupdate`, as browsers do (about 4 Hz).
const REPORT_MS: u64 = 250;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Has a source not loaded yet.
    Waiting,
    /// No source.
    Empty,
    Ready,
    Failed,
}

pub struct Media {
    pub node: NodeId,
    pub audio: bool,
    pub src: String,
    pub state: State,
    pub player: Option<Player>,
    pub want_play: bool,
    pub looped: bool,
    pub controls: bool,
    pub muted: bool,
    pub volume: u8,
    /// Events not yet told to the page, in order.
    pub events: Vec<&'static str>,
    last_report: u64,
    /// The picture size, once known.
    pub size: (usize, usize),
    /// Pictures shown, for logs and tests.
    pub frames: u32,
    /// The page feeds this element through a MediaSource.
    pub mse: Option<crate::mse::Mse>,
    /// The picture size became known while playing on (lay the page out again).
    pub resized: bool,
}

impl Media {
    pub fn new(dom: &Dom, node: NodeId) -> Media {
        let audio = dom.tag(node) == Some("audio");
        let src = source(dom, node);
        // A MediaSource's object URL is not loaded: the page attaches it ("mse").
        let waits = !src.is_empty() && !src.starts_with("blob:");
        Media {
            node, audio, state: if waits { State::Waiting } else { State::Empty }, src, player: None, mse: None, resized: false,
            want_play: dom.attr(node, "autoplay").is_some(), looped: dom.attr(node, "loop").is_some(),
            controls: dom.attr(node, "controls").is_some(), muted: dom.attr(node, "muted").is_some(), volume: 100,
            events: Vec::new(), last_report: 0, size: (0, 0), frames: 0,
        }
    }

    pub fn picture_name(&self) -> String {
        alloc::format!("{}:{}", if self.audio { "audio" } else { "video" }, self.node)
    }

    /// Opens a loaded body. True when the picture size became known (lay out again).
    pub fn open(&mut self, body: Vec<u8>) -> bool {
        let bytes: Rc<[u8]> = Rc::from(body);
        match Decoder::open(Box::new(bytes)) {
            Err(e) => {
                let mut text = [0u8; 80];
                sex_pdx::serial_record!("[web.media.failed] node={} err={} why={}", self.node, e.0, e.text(&mut text));
                self.state = State::Failed;
                self.events.push("error");
                false
            }
            Ok(dec) => self.start(dec, None),
        }
    }

    /// Plays `dec` (and `sound`, a separate sound stream, if any). True when the
    /// picture size became known.
    fn start(&mut self, dec: Decoder, sound: Option<Decoder>) -> bool {
        let info = dec.info.clone();
        let (w, h) = if self.audio { (0, 0) } else { sex_media::fit(info.width as usize, info.height as usize, MAX_W, MAX_H) };
        let mut player = match sound {
            Some(s) => Player::pair(dec, s, w, h, true, !self.want_play),
            None => Player::new(dec, w, h, true, !self.want_play),
        };
        let info = player.info().clone();
        player.set_volume(self.volume);
        player.set_muted(self.muted);
        sex_pdx::serial_record!("[web.media.opened] node={} format={} video={} audio={} w={} h={} ms={} sound={}",
            self.node, info.format, info.video_codec, info.audio_codec, w, h, info.duration_ms, player.has_sound() as u8);
        self.size = player.size();
        self.player = Some(player);
        self.state = State::Ready;
        self.events.extend_from_slice(&["durationchange", "loadedmetadata", "loadeddata", "canplay", "canplaythrough"]);
        if self.want_play { self.events.extend_from_slice(&["play", "playing"]); }
        self.size.0 > 0
    }

    /// Carries out a script's request. True if the element must be loaded (again).
    pub fn request(&mut self, dom: &Dom, verb: &str, arg: &str) -> bool {
        match verb {
            "play" => {
                self.want_play = true;
                if let Some(p) = self.player.as_mut() {
                    if p.paused() || p.ended() { p.play(); self.events.extend_from_slice(&["play", "playing"]); }
                }
            }
            "pause" => {
                self.want_play = false;
                if let Some(p) = self.player.as_mut() {
                    if !p.paused() { p.pause(); self.events.push("pause"); }
                }
            }
            "seek" => {
                let s: f64 = arg.parse().unwrap_or(0.0);
                if let Some(p) = self.player.as_mut() {
                    p.seek((s.max(0.0) * 1000.0) as u64);
                    self.events.extend_from_slice(&["seeking", "seeked", "timeupdate"]);
                }
            }
            "volume" => {
                let v: f64 = arg.parse().unwrap_or(1.0);
                self.volume = (v.clamp(0.0, 1.0) * 100.0) as u8;
                if let Some(p) = self.player.as_mut() { p.set_volume(self.volume); }
                self.events.push("volumechange");
            }
            "muted" => {
                self.muted = arg == "1";
                if let Some(p) = self.player.as_mut() { p.set_muted(self.muted); }
                self.events.push("volumechange");
            }
            "loop" => self.looped = arg == "1",
            "mse" => {
                self.player = None;
                self.mse = Some(crate::mse::Mse::new());
                self.state = State::Empty;
                self.events.push("emptied");
            }
            "add" | "duration" | "end" | "reopen" | "remove" | "drop" => {
                let node = self.node;
                let Some(m) = self.mse.as_mut() else { return false };
                match verb {
                    "add" => m.add(arg),
                    "duration" => { m.duration = arg.parse().unwrap_or(f64::NAN); self.events.push("durationchange"); }
                    "end" => m.end(),
                    "reopen" => m.reopen(),
                    "remove" => m.remove(node, arg),
                    _ => if let Some(b) = arg.parse::<usize>().ok().and_then(|i| m.buffers.get_mut(i)) { b.dropped = true; },
                }
            }
            "load" => {
                self.player = None;
                self.src = source(dom, self.node);
                self.state = if self.src.is_empty() { State::Empty } else { State::Waiting };
                self.events.push("emptied");
                self.want_play |= dom.attr(self.node, "autoplay").is_some();
                return self.state == State::Waiting;
            }
            _ => {}
        }
        false
    }

    /// Plays on: shows what is due into the page picture. Returns whether the picture
    /// changed and how soon to come back (None: nothing to do until something happens).
    pub fn step(&mut self, fonts: &mut Fonts) -> (bool, Option<u64>) {
        if self.player.is_none() && self.state != State::Failed && self.mse.as_ref().is_some_and(|m| m.openable()) {
            match self.mse.as_ref().unwrap().decoders() {
                Ok((Some(v), a)) => self.resized |= self.start(v, a),
                Ok((None, Some(a))) => self.resized |= self.start(a, None),
                Ok((None, None)) => {}
                Err(e) => {
                    let mut text = [0u8; 80];
                    sex_pdx::serial_record!("[web.mse.failed] node={} err={} why={}", self.node, e.0, e.text(&mut text));
                    self.state = State::Failed;
                    self.events.push("error");
                }
            }
        }
        let name = self.picture_name();
        let controls = self.controls;
        let Some(p) = self.player.as_mut() else { return (false, None) };
        let mut drew = false;
        let mut wait = None;
        loop {
            match p.step() {
                Step::Picture { .. } => { drew = true; self.frames += 1; }
                Step::Wait { ms } => { wait = Some(ms); break; }
                Step::Paused => break,
                Step::Ended => {
                    if self.looped {
                        p.seek(0);
                        p.play();
                        continue;
                    }
                    if self.want_play {
                        self.want_play = false;
                        self.events.extend_from_slice(&["timeupdate", "pause", "ended"]);
                        sex_pdx::serial_record!("[web.media.ended] node={} frames={} sound_frames={} played={}", self.node, self.frames, p.heard, p.played());
                    }
                    break;
                }
            }
        }
        let now = sex_pdx::get_ticks();
        if wait.is_some() && now >= self.last_report + REPORT_MS {
            self.last_report = now;
            self.events.push("timeupdate");
        }
        if drew || (controls && self.events.iter().any(|e| matches!(*e, "play" | "pause" | "timeupdate" | "seeked" | "ended"))) {
            publish(fonts, &name, p, controls, self.audio);
            return (true, wait.map(|w| w.min(REPORT_MS)));
        }
        (false, wait.map(|w| w.min(REPORT_MS)))
    }

    /// The `media` dispatch for the page: state, then the events since the last one.
    pub fn report(&mut self) -> Option<String> {
        if self.events.is_empty() { return None; }
        let events = self.events.join(",");
        self.events.clear();
        let (t, d, paused, ended, w, h, ready) = match self.player.as_mut() {
            Some(p) => {
                let d = p.duration_ms();
                (p.time_ms().max(0) as f64 / 1000.0, if d > 0 { alloc::format!("{}", d as f64 / 1000.0) } else { String::new() },
                 p.paused() || p.ended(), p.ended(), self.size.0, self.size.1, 4)
            }
            None => (0.0, String::new(), true, false, 0, 0, 0),
        };
        Some(alloc::format!("media\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", self.node, events, t, d, paused as u8, ended as u8, w, h, ready))
    }

    /// Space/K: play or pause.
    /// Whether a toggle would start it playing.
    pub fn stopped(&self) -> bool {
        self.player.as_ref().map_or(true, |p| p.paused() || p.ended())
    }

    /// Another element is about to play: give up the program's one sound ring, and
    /// pause if playing (only one element plays sound at a time).
    pub fn hush(&mut self) {
        let Some(p) = self.player.as_mut() else { return };
        if !p.has_sound() { return; }
        p.release_sound();
        if !p.paused() && !p.ended() {
            p.pause();
            self.events.push("pause");
        }
        self.want_play = false;
    }

    pub fn toggle(&mut self, dom: &Dom) {
        let playing = self.player.as_ref().is_some_and(|p| !p.paused() && !p.ended());
        self.request(dom, if playing { "pause" } else { "play" }, "");
    }

    pub fn seek_by(&mut self, dom: &Dom, ms: i64) {
        let Some(p) = self.player.as_mut() else { return };
        let to = (p.time_ms() + ms).max(0) as f64 / 1000.0;
        self.request(dom, "seek", &alloc::format!("{to}"));
    }

    /// Draws the element's picture: a poster or nothing until it plays.
    pub fn poster(fonts: &mut Fonts, name: &str, img: Image) {
        put(fonts, name, img.width, img.height, |px| px.copy_from_slice(&img.pixels));
    }
}

/// The address an element plays: its `src`, or its first `<source>` of a kind sex-media
/// can play.
fn source(dom: &Dom, node: NodeId) -> String {
    if let Some(src) = dom.attr(node, "src").filter(|s| !s.trim().is_empty()) {
        return String::from(src.trim());
    }
    for &c in &dom.nodes[node].children {
        if dom.tag(c) != Some("source") { continue; }
        let Some(src) = dom.attr(c, "src").filter(|s| !s.trim().is_empty()) else { continue };
        let ty = dom.attr(c, "type").unwrap_or("").to_ascii_lowercase();
        let playable = ty.is_empty() || ["mp4", "webm", "ogg", "mpeg", "mp3", "aac", "flac", "wav", "x-m4a", "quicktime", "matroska"].iter().any(|t| ty.contains(t));
        if playable { return String::from(src.trim()); }
    }
    String::new()
}

/// The video and audio elements in the tree, in document order.
pub fn elements(dom: &Dom) -> Vec<NodeId> {
    dom.descendants(Dom::ROOT).into_iter().filter(|&n| matches!(dom.tag(n), Some("video" | "audio"))).collect()
}

/// Writes the page picture `name`, in place when nothing else holds it.
fn put(fonts: &mut Fonts, name: &str, w: usize, h: usize, fill: impl FnOnce(&mut [u32])) {
    let i = fonts.images.iter().position(|(s, _)| s == name);
    if let Some(i) = i {
        let img = &mut fonts.images[i].1;
        if img.width == w && img.height == h {
            if let Some(img) = Rc::get_mut(img) {
                fill(&mut img.pixels);
                return;
            }
        }
    }
    let mut pixels = alloc::vec![0xFF00_0000u32; w * h];
    fill(&mut pixels);
    let img = Rc::new(Image { width: w, height: h, pixels });
    match i {
        Some(i) => fonts.images[i].1 = img,
        None => fonts.images.push((String::from(name), img)),
    }
}

/// The current picture, with the control bar over it when the element has `controls`.
/// A sound element's picture is the bar alone.
fn publish(fonts: &mut Fonts, name: &str, p: &mut Player, controls: bool, audio: bool) {
    let playing = !p.paused() && !p.ended();
    let (t, d) = (p.time_ms().max(0) as u64, p.duration_ms());
    if audio {
        put(fonts, name, 300, 32, |px| {
            px.fill(0xFFF1_F3F4);
            bar(px, 300, 32, 0, 32, playing, t, d, false);
        });
        return;
    }
    let (w, h) = p.size();
    let pic = p.picture();
    if pic.len() < w * h || w == 0 { return; }
    put(fonts, name, w, h, |px| {
        px.copy_from_slice(&pic[..w * h]);
        if controls && h > BAR_H * 2 { bar(px, w, h, h - BAR_H, BAR_H, playing, t, d, true); }
    });
}

/// A control bar `rows` high at row `top`: the play or pause sign, then the progress.
/// Over a picture it darkens what is under it.
#[allow(clippy::too_many_arguments)]
fn bar(px: &mut [u32], w: usize, h: usize, top: usize, rows: usize, playing: bool, t: u64, d: u64, over: bool) {
    let rows = rows.min(h - top);
    for y in top..top + rows {
        for p in &mut px[y * w..(y + 1) * w] {
            *p = if over { 0xFF00_0000 | ((*p >> 2) & 0x3F_3F3F) } else { 0xFFDD_DFE1 };
        }
    }
    let ink = if over { 0xFFFF_FFFF } else { 0xFF3C_4043 };
    let mid = top + rows / 2;
    // The sign, 10 px tall, at the left.
    for dy in 0..10usize {
        let y = mid - 5 + dy;
        if y >= h { continue; }
        let row = &mut px[y * w..(y + 1) * w];
        if playing {
            for x in [8usize, 9, 10, 13, 14, 15] { if x < w { row[x] = ink; } }
        } else {
            let half = if dy < 5 { dy } else { 9 - dy };
            for x in 8..(9 + half * 2).min(w) { row[x] = ink; }
        }
    }
    // Progress: a track and how far along it is.
    let (x0, x1) = (24usize, w.saturating_sub(8));
    if x1 <= x0 { return; }
    let done = if d > 0 { x0 + ((x1 - x0) as u64 * t.min(d) / d) as usize } else { x0 };
    for y in mid - 1..=mid + 1 {
        if y >= h { continue; }
        for x in x0..x1 { px[y * w + x] = if x < done { 0xFF1A_73E8 } else if over { 0xFF80_8080 } else { 0xFF9A_A0A6 }; }
    }
}

/// AVIF pictures for `<img>`: sex-image reads no AV1, so they go through FFmpeg's dav1d.
pub fn decode_avif(bytes: &[u8], max_w: usize, max_h: usize) -> Option<Image> {
    match sex_media::decode_picture(bytes, max_w, max_h) {
        Ok((width, height, pixels)) => {
            let centre = pixels[(height / 2) * width + width / 2];
            sex_pdx::serial_record!("[web.avif] w={} h={} bytes={} centre={:06x}", width, height, bytes.len(), centre & 0xFF_FFFF);
            Some(Image { width, height, pixels })
        }
        Err(e) => {
            let mut buf = [0u8; 64];
            sex_pdx::serial_record!("[web.avif] failed: {}", e.text(&mut buf));
            None
        }
    }
}
