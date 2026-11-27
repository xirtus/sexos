// SPDX-License-Identifier: GPL-3.0-only
//! Splicy: the SexOS video editor. `run splicy @talk.webm` opens the clip with its
//! transcript `talk.words` beside it, and editing the transcript edits the video.
//! The clip is only read; Ctrl+E writes the edit to `talk-edit.webm` (VP9 and Opus).
//!
//! Keys: Left/Right move word by word and Up/Down a line's worth, with Shift to select;
//! a click puts the caret on a word. Delete or Backspace cuts
//! the selection from picture and sound together. S selects the caret's sentence. C runs
//! the clean-up pass (first takes, filler words, long pauses) as suggestion cards: Tab
//! moves between them, Enter takes one and Esc skips it. `\` plays the pictures two
//! seconds either side of the nearest join. Ctrl+Z and Ctrl+Y undo and redo.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(not(feature = "sexos"))]
fn main() {}

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;

    use alloc::boxed::Box;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use sex_app::sound::Sound;
    use sex_app::{files, Event, Nav, Window};
    use sex_media::export::{Config, Exporter, Format, RATE};
    use sex_media::recording::Drain;
    use sex_media::{Decoder, Next, Source};
    use sex_pdx::app_key::Key;
    use sex_pdx::app_pointer::Kind;
    use sex_splice::export::{export_pictures, sound_pieces, Frames, Pictures, SoundPiece};
    use sex_splice::{Micros, TrackKind, SECOND};
    use splicy::cache::{CacheConfig, CachedFrame, FrameCache};
    use splicy::media::{Library, Peaks, Thumb};
    use splicy::view::{CardHit, ToolbarHit, TrackSwitchHit, TransportHit, View};
    use splicy::{words, AuditionMode, Editor, ToolKind};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[splicy.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    /// The clip, read through Files a piece at a time. It holds no Window: reads go
    /// straight to Files, so events stay queued on the program's own window.
    struct FileSource {
        path: Vec<u8>,
        len: u64,
        cache: Vec<u8>,
        cache_at: u64,
    }

    impl Source for FileSource {
        fn len(&self) -> Option<u64> {
            Some(self.len)
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
            if at >= self.len {
                return Some(0);
            }
            if !(at >= self.cache_at && at < self.cache_at + self.cache.len() as u64) {
                let want = (256 * 1024u64).min(self.len - at) as usize;
                self.cache.resize(want, 0);
                match files::direct::read_at(&self.path, at, &mut self.cache) {
                    Ok(got) if got > 0 => {
                        self.cache.truncate(got);
                        self.cache_at = at;
                    }
                    _ => return None,
                }
            }
            let off = (at - self.cache_at) as usize;
            let n = out.len().min(self.cache.len() - off);
            out[..n].copy_from_slice(&self.cache[off..off + n]);
            Some(n)
        }
    }

    fn open_decoder(path: &[u8], len: u64) -> Result<Decoder, sex_media::Error> {
        Decoder::open(Box::new(FileSource { path: path.to_vec(), len, cache: Vec::new(), cache_at: 0 }))
    }

    /// The picture a source shows at a time: decodes forward, seeking only when asked
    /// for a time behind it or more than two seconds ahead.
    struct Grabber {
        dec: Decoder,
        w: usize,
        h: usize,
        picture: Vec<u32>,
        /// Time of the picture held, in ms; -1 before the first.
        at_ms: i64,
        frame_ms: i64,
        pcm: Vec<[i16; 2]>,
        cache: FrameCache,
    }

    impl Grabber {
        fn show(&mut self, source: Micros) -> Result<(), sex_media::Error> {
            self.show_mode(source, true)
        }

        fn show_mode(&mut self, source: Micros, cache_frame: bool) -> Result<(), sex_media::Error> {
            let want = source / 1000;
            if self.at_ms >= 0 && want >= self.at_ms && want < self.at_ms + self.frame_ms {
                return Ok(());
            }
            if cache_frame {
                let tol_us = (self.frame_ms * 500).max(500);
                if let Some(hit) = self.cache.get(0, source, tol_us, false) {
                    if hit.pixels.len() == self.picture.len() {
                        self.picture.copy_from_slice(&hit.pixels);
                        self.at_ms = hit.source_us / 1000;
                        return Ok(());
                    }
                }
            }
            if self.at_ms < 0 || want < self.at_ms || want > self.at_ms + 2000 {
                self.dec.seek(want.max(0) as u64)?;
            }
            let started = sex_pdx::get_ticks();
            for _ in 0..2000 {
                match self.dec.next(&mut self.picture, self.w, self.h, &mut self.pcm)? {
                    Next::Picture { at_ms } => {
                        self.at_ms = at_ms.max(0);
                        if at_ms + self.frame_ms / 2 >= want {
                            if cache_frame {
                                self.cache.insert(CachedFrame::new(
                                    0,
                                    self.at_ms * 1000,
                                    self.w,
                                    self.h,
                                    false,
                                    self.picture.clone(),
                                ));
                            }
                            return Ok(());
                        }
                        // During live playback, yield once decode budget exceeds 24 ms so UI & audio never starve.
                        if !cache_frame && sex_pdx::get_ticks().saturating_sub(started) >= 24 {
                            return Ok(());
                        }
                    }
                    Next::End => return Ok(()),
                    _ => {}
                }
            }
            Ok(())
        }
    }

    impl Pictures for Grabber {
        type Error = sex_media::Error;
        fn picture(&mut self, _media: u32, source: Micros, out: &mut [u32]) -> Result<(), Self::Error> {
            self.show(source)?;
            out.copy_from_slice(&self.picture[..out.len()]);
            Ok(())
        }
    }

    /// Lays the edit's sound into the export, from a second decoder of the same clip so
    /// pictures and sound each read forward.
    struct SoundFeed {
        dec: Option<Decoder>,
        pieces: Vec<SoundPiece>,
        piece: usize,
        /// Output samples written so far.
        written: u64,
        /// Decoded source samples not yet used, starting at source sample `held_at` (in
        /// output-rate samples).
        held: Vec<[i16; 2]>,
        held_at: u64,
        scratch: Vec<[i16; 2]>,
    }

    impl SoundFeed {
        /// The source samples starting at `from` (output rate), at least one, or none at
        /// the end of the clip.
        fn source(&mut self, from: u64) -> Result<bool, sex_media::Error> {
            let Some(dec) = self.dec.as_mut() else { return Ok(false) };
            let have = from >= self.held_at && from < self.held_at + self.held.len() as u64;
            if have {
                return Ok(true);
            }
            let ahead = from >= self.held_at + self.held.len() as u64 && from < self.held_at + self.held.len() as u64 + RATE as u64;
            if !ahead {
                dec.seek(from * 1000 / RATE as u64)?;
            }
            let mut tiny = [0u32; 4];
            for _ in 0..4000 {
                match dec.next(&mut tiny, 2, 2, &mut self.scratch)? {
                    Next::Sound { frames, at_ms } => {
                        let start = at_ms.max(0) as u64 * RATE as u64 / 1000;
                        // The decoder already gives 48 kHz stereo, whatever the source rate.
                        self.held.clear();
                        self.held.extend_from_slice(&self.scratch[..frames.min(self.scratch.len())]);
                        self.held_at = start;
                        if from < start + self.held.len() as u64 && from + RATE as u64 / 10 >= start {
                            return Ok(true);
                        }
                    }
                    Next::End => return Ok(false),
                    _ => {}
                }
            }
            Ok(false)
        }

        /// Write sound up to output sample `until`: the pieces' samples, silence between.
        fn fill(&mut self, ex: &mut Exporter, window: &mut Window, path: &[u8], until: u64) -> Result<(), &'static str> {
            let mut out: Vec<[i16; 2]> = Vec::with_capacity(4096);
            while self.written < until {
                out.clear();
                let p = self.pieces.get(self.piece).copied();
                match p {
                    Some(p) if self.written >= p.at + p.samples => {
                        self.piece += 1;
                        continue;
                    }
                    Some(p) if self.written >= p.at => {
                        let from = p.source + (self.written - p.at);
                        let room = (p.at + p.samples).min(until) - self.written;
                        if self.source(from).map_err(|_| "Cannot read the clip's sound.")? && from >= self.held_at {
                            let off = (from - self.held_at) as usize;
                            let n = (room as usize).min(self.held.len() - off).min(4096);
                            for s in &self.held[off..off + n] {
                                let v = |x: i16| (x as i32 * p.volume as i32 / 1000).clamp(-32768, 32767) as i16;
                                out.push([v(s[0]), v(s[1])]);
                            }
                        }
                        if out.is_empty() {
                            out.resize((room as usize).min(4096), [0, 0]);
                        }
                    }
                    Some(p) => out.resize(((p.at.min(until) - self.written) as usize).min(4096), [0, 0]),
                    None => out.resize(((until - self.written) as usize).min(4096), [0, 0]),
                }
                ex.sound(&out).map_err(|_| "The encoder refused the sound.")?;
                drain(ex, window, path)?;
                self.written += out.len() as u64;
            }
            Ok(())
        }
    }

    fn drain(ex: &mut Exporter, window: &mut Window, path: &[u8]) -> Result<bool, &'static str> {
        loop {
            match ex.output().map_err(|_| "The encoder failed.")? {
                Drain::NeedFrame => return Ok(false),
                Drain::End => return Ok(true),
                Drain::Write(w) => {
                    let (token, offset, n) = (w.token, w.offset, w.bytes.len());
                    let wrote = files::write_at(window, path, offset, w.bytes).map_err(|_| "Cannot write the export.")?;
                    if wrote != n {
                        return Err("The disk is full.");
                    }
                    ex.acknowledge(token, wrote).map_err(|_| "The encoder failed.")?;
                }
            }
        }
    }

    /// Takes the exported pictures, and the sound up to each one.
    struct Sink<'a> {
        ex: &'a mut Exporter,
        window: &'a mut Window,
        path: &'a [u8],
        feed: Option<&'a mut SoundFeed>,
        /// The edit's length in output samples: sound never runs past it.
        total: u64,
        fps: (u32, u32),
        frames: u64,
    }

    impl Frames for Sink<'_> {
        type Error = &'static str;
        fn frame(&mut self, pixels: &[u32], at: Micros) -> Result<(), Self::Error> {
            let n = ((at as i128 * self.fps.0 as i128 + (self.fps.1 as i128 * SECOND as i128) / 2) / (self.fps.1 as i128 * SECOND as i128)) as u64;
            self.ex.picture(pixels, n).map_err(|e| {
                sex_pdx::serial_record!("[splicy.export.picture.error] frame={} error={:?} pixels={}", n, e, pixels.len());
                "The encoder refused a picture."
            })?;
            drain(self.ex, self.window, self.path)?;
            if let Some(feed) = self.feed.as_deref_mut() {
                let until = ((at as u64 * RATE as u64 / SECOND as u64) + RATE as u64 / 10).min(self.total);
                feed.fill(self.ex, self.window, self.path, until)?;
            }
            self.frames += 1;
            if self.frames % 30 == 0 {
                sex_pdx::serial_record!("[splicy.export.progress] frames={}", self.frames);
            }
            Ok(())
        }
    }

    /// Zero-seek RAM-backed 48 kHz stereo audio engine over SexOS `Bell` (`Sound`).
    /// Reads decoded samples straight from `Library::pcm` through `sound_pieces(tl, RATE)`
    /// with a 750 ms ring lookahead and hardware `played_total()` clock lock.
    struct LiveAudio {
        sound: Sound,
        pieces: Vec<SoundPiece>,
        piece: usize,
        written: u64,
        base_us: Micros,
        played_base: u64,
        last_played: u64,
        moved_tick: u64,
        version_idx: usize,
        rate: i8,
        active: bool,
        scratch: Vec<[i16; 2]>,
    }

    impl LiveAudio {
        fn open(tl: &sex_splice::Timeline, version_idx: usize) -> Option<Self> {
            let sound = Sound::open()?;
            let now = sex_pdx::get_ticks();
            Some(Self {
                sound,
                pieces: sound_pieces(tl, RATE),
                piece: 0,
                written: 0,
                base_us: 0,
                played_base: 0,
                last_played: 0,
                moved_tick: now,
                version_idx,
                rate: 0,
                active: false,
                scratch: Vec::with_capacity(4096),
            })
        }

        fn invalidate(&mut self) {
            self.active = false;
        }

        fn stop(&mut self) {
            if self.active {
                self.sound.pause(true);
                self.sound.skip();
                self.active = false;
                self.rate = 0;
            }
        }

        fn feed(&mut self, tl: &sex_splice::Timeline, lib: &Library, version_idx: usize, playhead: Micros, rate: i8) {
            if rate <= 0 {
                self.stop();
                return;
            }
            if !self.active || self.version_idx != version_idx || self.rate != rate {
                self.sound.reopen();
                self.sound.set_volume(100);
                let ring_hz = if rate == 1 { 0 } else { RATE * (rate as u32) };
                self.sound.set_rate(ring_hz);
                self.sound.pause(false);
                self.sound.skip();
                self.played_base = self.sound.written_total();
                self.last_played = self.sound.played_total();
                self.moved_tick = sex_pdx::get_ticks();
                self.base_us = playhead.max(0);
                self.pieces = sound_pieces(tl, RATE);
                let start_sample = (playhead.max(0) as u64 * RATE as u64) / SECOND as u64;
                self.written = start_sample;
                self.piece = self
                    .pieces
                    .iter()
                    .position(|p| p.at + p.samples > start_sample)
                    .unwrap_or(self.pieces.len());
                self.version_idx = version_idx;
                self.rate = rate;
                self.active = true;
            }

            let total = (tl.duration().max(0) as u64 * RATE as u64) / SECOND as u64;
            let current_sample = (playhead.max(0) as u64 * RATE as u64) / SECOND as u64;
            let lookahead = (RATE as u64 * 750 / 1000) * (rate as u64);
            let until = (current_sample + lookahead).min(total);

            while self.written < until && self.sound.room() >= 256 {
                self.scratch.clear();
                let max_chunk = self.sound.room().min(4096);
                let p = self.pieces.get(self.piece).copied();
                match p {
                    Some(p) if self.written >= p.at + p.samples => {
                        self.piece += 1;
                        continue;
                    }
                    Some(p) if self.written >= p.at => {
                        let from = (p.source + (self.written - p.at)) as usize;
                        let room = ((p.at + p.samples).min(until) - self.written) as usize;
                        let want = room.min(max_chunk);
                        if let Some(src_pcm) = lib.pcm.get(&p.media) {
                            if from < src_pcm.len() {
                                let avail = want.min(src_pcm.len() - from);
                                if p.volume == 1000 {
                                    self.scratch.extend_from_slice(&src_pcm[from..from + avail]);
                                } else {
                                    let vol = p.volume as i32;
                                    for s in &src_pcm[from..from + avail] {
                                        let l = (s[0] as i32 * vol / 1000).clamp(-32768, 32767) as i16;
                                        let r = (s[1] as i32 * vol / 1000).clamp(-32768, 32767) as i16;
                                        self.scratch.push([l, r]);
                                    }
                                }
                            }
                        }
                        if self.scratch.len() < want {
                            self.scratch.resize(want.max(1), [0, 0]);
                        }
                    }
                    Some(p) => {
                        let gap = ((p.at.min(until) - self.written) as usize).min(max_chunk);
                        self.scratch.resize(gap.max(1), [0, 0]);
                    }
                    None => {
                        let rem = ((until - self.written) as usize).min(max_chunk);
                        self.scratch.resize(rem.max(1), [0, 0]);
                    }
                }
                let wrote = self.sound.write(&self.scratch);
                if wrote == 0 {
                    break;
                }
                self.written += wrote as u64;
            }
        }

        fn clock_us(&mut self) -> Option<Micros> {
            if !self.active || self.rate <= 0 {
                return None;
            }
            let now_tick = sex_pdx::get_ticks();
            let played = self.sound.played_total();
            if played != self.last_played {
                self.last_played = played;
                self.moved_tick = now_tick;
            }
            if played <= self.played_base {
                return None;
            }
            let pending = self.sound.written_total().saturating_sub(played);
            if pending > 0 && now_tick.saturating_sub(self.moved_tick) > 750 {
                return None;
            }
            let frames_heard = played - self.played_base;
            let heard_us = (frames_heard as i128 * SECOND as i128 / RATE as i128) as Micros;
            let interp_ms = now_tick.saturating_sub(self.moved_tick).min(43) as i64;
            let interp_us = interp_ms * 1000 * (self.rate as i64);
            Some(self.base_us + heard_us + interp_us)
        }
    }

    struct App {
        window: Window,
        path: Vec<u8>,
        len: u64,
        name: String,
        ed: Editor,
        view: View,
        grab: Option<Grabber>,
        audio: Option<LiveAudio>,
        lib: Library,
        has_sound: bool,
        playhead: Micros,
        last_tick_ms: u64,
        next_frame_tick: u64,
        dragging_timeline: bool,
        dragging_text: bool,
        message: String,
    }

    impl App {
        fn sync_audio(&mut self) {
            if let Some(audio) = self.audio.as_mut() {
                let ver = self.ed.history.head();
                audio.feed(self.ed.timeline(), &self.lib, ver, self.playhead, self.ed.shuttle_rate);
            }
        }

        fn seek_playhead(&mut self, to: Micros) {
            self.playhead = to.clamp(0, self.ed.timeline().duration());
            if let Some(audio) = self.audio.as_mut() {
                audio.invalidate();
            }
        }

        fn stop_playback(&mut self) {
            self.ed.shuttle_pause();
            if let Some(audio) = self.audio.as_mut() {
                audio.stop();
            }
        }

        fn toggle_play_pause(&mut self) {
            let dur = self.ed.timeline().duration();
            if self.ed.shuttle_rate == 0 {
                let stop_at = self.ed.out_point.filter(|&o| o > self.ed.in_point.unwrap_or(0)).unwrap_or(dur);
                if self.playhead >= stop_at || self.playhead >= dur {
                    self.playhead = self.ed.in_point.unwrap_or(0).clamp(0, dur);
                }
                if let Some(audio) = self.audio.as_mut() {
                    audio.invalidate();
                }
                self.ed.shuttle_rate = 1;
                let now = sex_pdx::get_ticks();
                self.last_tick_ms = now;
                self.next_frame_tick = now;
                self.message = "Playing (1x)".into();
            } else {
                self.stop_playback();
                self.message = "Paused".into();
            }
            self.sync_audio();
        }

        fn step_frames(&mut self, frames: i64) {
            self.stop_playback();
            self.seek_playhead(self.playhead + self.frame_step_us() * frames);
            if let Some(i) = self.ed.word_at_time(self.playhead) {
                self.ed.place(i, false);
            }
        }

        fn run_cleanup(&mut self) {
            let n = self.ed.suggest_cleanup_with(self.lib.levels.get(&self.ed.transcript.media));
            sex_pdx::serial_record!("[splicy.cleanup] cards={}", n);
            self.message = format!("{n} suggestion{} to look at", if n == 1 { "" } else { "s" });
        }

        fn draw(&mut self) {
            self.sync_audio();
            let playing = self.ed.shuttle_rate != 0;
            let v_enabled = self
                .ed
                .timeline()
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Video)
                .and_then(|t| t.clips.iter().find(|c| c.at <= self.playhead && self.playhead < c.end()))
                .is_none_or(|c| c.enabled);
            if v_enabled {
                if let (Some(g), Some((media, s))) = (self.grab.as_mut(), sex_splice::export::picture_at(self.ed.timeline(), self.playhead)) {
                    let _ = media;
                    let _ = g.show_mode(s, !playing);
                }
            }
            if self.view.vu.visible {
                self.view.vu.update_from_library(self.ed.timeline(), &self.lib, self.playhead, 40_000);
            }
            let (w, h) = self.window.size();
            let picture = if v_enabled {
                self.grab.as_ref().map(|g| (&g.picture[..], g.w, g.h))
            } else {
                None
            };
            self.view.paint(self.window.pixels(), w, h, &self.ed, &self.lib, picture, self.playhead, &self.message);
            self.window.show_pixels(w, h);
        }

        fn report(&self) {
            let tl = self.ed.timeline();
            let kept = self.ed.playing().iter().filter(|p| **p).count();
            sex_pdx::serial_record!(
                "[splicy.state] words={} playing={} duration_ms={} clips={} joins={} versions={} pending={} caret={}",
                self.ed.transcript.words.len(),
                kept,
                tl.duration() / 1000,
                tl.tracks.iter().map(|t| t.clips.len()).sum::<usize>(),
                self.ed.joins().len(),
                self.ed.history.versions().len(),
                self.ed.pending().len(),
                self.ed.caret
            );
        }

        fn follow_caret(&mut self) {
            if let Some(t) = self.ed.caret_time() {
                self.seek_playhead(t);
            }
        }

        fn frame_step_us(&self) -> Micros {
            let tl = self.ed.timeline();
            (tl.fps_den as i64 * SECOND / tl.fps_num.max(1) as i64).max(16_000)
        }

        fn step_shuttle(&mut self) {
            let rate = self.ed.shuttle_rate;
            if rate == 0 {
                return;
            }
            let now_ms = sex_pdx::get_ticks();
            let dt_ms = now_ms.saturating_sub(self.last_tick_ms).clamp(1, 200);
            self.last_tick_ms = now_ms;
            let dur = self.ed.timeline().duration();
            let stop_end = match (self.ed.in_point, self.ed.out_point) {
                (_, Some(out)) if rate > 0 && self.playhead < out => out.min(dur),
                _ => dur,
            };
            let stop_start = match self.ed.in_point {
                Some(inp) if rate < 0 && self.playhead > inp => inp.max(0),
                _ => 0,
            };
            self.sync_audio();
            let raw_next = if let Some(hw_us) = self.audio.as_mut().and_then(|a| a.clock_us()) {
                hw_us.max(self.playhead)
            } else {
                self.playhead + (dt_ms as i64) * 1000 * (rate as i64)
            };
            let next = raw_next.clamp(stop_start, stop_end);
            self.playhead = next;
            if let Some(i) = self.ed.word_at_time(next) {
                self.ed.place(i, false);
            }
            if (rate < 0 && next <= stop_start) || (rate > 0 && next >= stop_end) {
                self.stop_playback();
            }
        }

        fn key(&mut self, key: Key) {
            self.message.clear();
            let ch = sex_app::typed(Key { control: false, shift: false, alt: false, ..key });
            if key.control {
                self.stop_playback();
                match ch {
                    Some(b'z') => {
                        self.ed.undo();
                    }
                    Some(b'y') => {
                        self.ed.redo();
                    }
                    Some(b'e') => self.export(),
                    Some(b'h') => self.export_handoff(),
                    _ => return,
                }
                self.report();
                return;
            }
            if let Some(nav) = sex_app::nav(key) {
                self.stop_playback();
                match nav {
                    Nav::Left => self.ed.move_caret(-1, key.shift),
                    Nav::Right => self.ed.move_caret(1, key.shift),
                    Nav::Up => self.ed.move_caret(-8, key.shift),
                    Nav::Down => self.ed.move_caret(8, key.shift),
                    Nav::Home => self.ed.move_caret(isize::MIN / 2, key.shift),
                    Nav::End => self.ed.move_caret(isize::MAX / 2, key.shift),
                    Nav::Delete => self.cut(),
                }
                self.follow_caret();
                return;
            }
            match (key.code, ch) {
                (0x39, _) | (_, Some(b' ')) => self.toggle_play_pause(),
                (0x0E, _) => {
                    self.stop_playback();
                    self.cut();
                }
                (0x0F, _) => self.ed.next_card(),
                (0x1C, _) => match self.ed.accept() {
                    Ok(true) => {
                        sex_pdx::serial_record!("[splicy.suggestion] taken=1");
                        self.report();
                    }
                    Ok(false) => {}
                    Err(_) => self.message = "That suggestion no longer fits the edit.".into(),
                },
                (0x01, _) => {
                    if self.ed.skip() {
                        sex_pdx::serial_record!("[splicy.suggestion] skipped=1");
                    } else if self.ed.shuttle_rate != 0 {
                        self.stop_playback();
                        self.message = "Paused".into();
                    }
                }
                (_, Some(b's')) => self.ed.select_sentence(),
                (_, Some(b'c')) => self.run_cleanup(),
                (_, Some(b'm')) => {
                    if let Some(ti) = self.ed.timeline().tracks.iter().position(|t| t.kind == TrackKind::Audio) {
                        let muted = self.ed.toggle_track_muted(ti);
                        self.message = if muted { "Audio track muted".into() } else { "Audio track unmuted".into() };
                        self.report();
                    }
                }
                (_, Some(b',')) => self.step_frames(-1),
                (_, Some(b'.')) => self.step_frames(1),
                (_, Some(b'v')) if key.shift => match self.ed.toggle_clip_enabled_at(self.playhead) {
                    Ok(true) => {
                        self.message = "Toggled clip enabled state".into();
                        self.report();
                    }
                    Ok(false) => self.message = "No clip under playhead.".into(),
                    Err(_) => self.message = "Cannot toggle clip there.".into(),
                },
                (_, Some(b'v')) => {
                    self.view.vu.visible = !self.view.vu.visible;
                    self.message = if self.view.vu.visible { "VU meter strip on".into() } else { "VU meter strip off".into() };
                }
                (_, Some(b'\\')) => {
                    self.stop_playback();
                    self.preview_join();
                }
                (_, Some(c @ (b'j' | b'l'))) if key.alt => {
                    self.stop_playback();
                    // Alt+J / Alt+L: split edit (J cut / L cut).
                    let shift = if c == b'j' { -SECOND / 4 } else { SECOND / 4 };
                    match self.ed.split_edit(shift) {
                        Ok(at) => {
                            sex_pdx::serial_record!("[splicy.split_edit] kind={} at_ms={}", c as char, at / 1000);
                            self.message = format!("{} cut: the sound now cuts a quarter second {} the picture", if c == b'j' { "J" } else { "L" }, if c == b'j' { "before" } else { "after" });
                            self.report();
                        }
                        Err(e) => self.message = format!("No split edit there: {e:?}"),
                    }
                }
                (_, Some(b'j')) => {
                    let rate = self.ed.shuttle_reverse();
                    let now = sex_pdx::get_ticks();
                    self.last_tick_ms = now;
                    self.next_frame_tick = now;
                    if let Some(audio) = self.audio.as_mut() {
                        audio.invalidate();
                    }
                    self.sync_audio();
                    self.message = format!("Shuttle {}x reverse", -rate);
                }
                (_, Some(b'k')) => {
                    self.stop_playback();
                    self.message = "Paused (0x)".into();
                }
                (_, Some(b'l')) => {
                    let dur = self.ed.timeline().duration();
                    if self.ed.shuttle_rate == 0 && self.playhead >= dur {
                        self.playhead = self.ed.in_point.unwrap_or(0);
                    }
                    let rate = self.ed.shuttle_forward();
                    let now = sex_pdx::get_ticks();
                    self.last_tick_ms = now;
                    self.next_frame_tick = now;
                    if let Some(audio) = self.audio.as_mut() {
                        audio.invalidate();
                    }
                    self.sync_audio();
                    self.message = format!("Shuttle {rate}x forward");
                }
                (_, Some(b'i')) => {
                    self.ed.mark_in(self.playhead);
                    self.message = "Marked In point".into();
                }
                (_, Some(b'o')) => {
                    self.ed.mark_out(self.playhead);
                    self.message = "Marked Out point".into();
                }
                (_, Some(b'x')) => {
                    self.ed.clear_in_out();
                    self.message = "Cleared In/Out marks".into();
                }
                (_, Some(b'b')) => match self.ed.blade_at(self.playhead) {
                    Ok(true) => {
                        self.message = "Blade cut at playhead".into();
                        self.report();
                    }
                    Ok(false) => self.message = "No clip spans the playhead.".into(),
                    Err(_) => self.message = "Cannot blade there.".into(),
                },
                (_, Some(b'a')) => {
                    self.ed.active_tool = ToolKind::Select;
                    self.message = "Select tool (A)".into();
                }
                (_, Some(b't')) => {
                    self.ed.active_tool = ToolKind::Trim;
                    self.message = "Trim tool (T)".into();
                }
                (_, Some(b'p')) => {
                    self.view.layout = self.view.layout.toggle();
                    self.message = match self.view.layout {
                        splicy::LayoutMode::FinalCut => "Layout: Final Cut Pro".into(),
                        splicy::LayoutMode::Premiere => "Layout: Premiere".into(),
                    };
                }
                (_, Some(b'=')) => {
                    let w = self.window.size().0 as i32;
                    self.view.zoom_by(self.ed.timeline(), w, 1, self.playhead);
                }
                (_, Some(b'-')) => {
                    let w = self.window.size().0 as i32;
                    self.view.zoom_by(self.ed.timeline(), w, -1, self.playhead);
                }
                _ => {}
            }
        }

        fn cut(&mut self) {
            let sel = self.ed.selection();
            match self.ed.delete() {
                Ok(true) => {
                    if let Some((a, b)) = sel {
                        sex_pdx::serial_record!("[splicy.cut] first={} last={} text={}", a, b, self.ed.quote(a, b));
                    }
                    self.ed.audition = AuditionMode::HearJoin;
                    self.follow_caret();
                    self.report();
                }
                Ok(false) => self.message = "Those words are already cut.".into(),
                Err(_) => self.message = "Cannot cut there.".into(),
            }
        }

        /// Two-state `\` audition: alternates between playing the join and playing what was cut.
        fn preview_join(&mut self) {
            let mode = self.ed.audition;
            if mode == AuditionMode::HearRemoved {
                if let Some((_media, removed)) = self.ed.removed_at_nearest_join() {
                    sex_pdx::serial_record!("[splicy.join.removed] from_ms={} to_ms={}", removed.start / 1000, removed.end / 1000);
                    self.message = "Auditioning removed passage (\\ again for the join)".into();
                    let tl = self.ed.timeline();
                    let step = (tl.fps_den as i64 * SECOND / tl.fps_num.max(1) as i64).max(1);
                    let begun = sex_pdx::get_ticks();
                    let mut s = removed.start;
                    while s < removed.end {
                        if let Some(Event::Close) = self.window.poll() {
                            self.window.close_now();
                        }
                        if let Some(g) = self.grab.as_mut() {
                            let _ = g.show(s);
                        }
                        let (w, h) = self.window.size();
                        let picture = self.grab.as_ref().map(|g| (&g.picture[..], g.w, g.h));
                        self.view.paint(self.window.pixels(), w, h, &self.ed, &self.lib, picture, self.playhead, &self.message);
                        self.window.show_pixels(w, h);
                        let due = begun + ((s - removed.start + step) / 1000) as u64;
                        let now = sex_pdx::get_ticks();
                        if due > now {
                            let _ = sex_pdx::sys_wait_self_ticks(due - now);
                        }
                        s += step;
                    }
                    self.ed.cycle_audition();
                    self.follow_caret();
                    return;
                }
            }
            let Some(r) = self.ed.join_preview() else {
                self.message = "No cuts yet: delete some words first.".into();
                return;
            };
            sex_pdx::serial_record!("[splicy.join] from_ms={} to_ms={}", r.start / 1000, r.end / 1000);
            self.message = "Auditioning join (\\ again for removed footage)".into();
            let tl = self.ed.timeline();
            let step = (tl.fps_den as i64 * SECOND / tl.fps_num.max(1) as i64).max(1);
            let begun = sex_pdx::get_ticks();
            let mut t = r.start;
            while t < r.end {
                if let Some(Event::Close) = self.window.poll() {
                    self.window.close_now();
                }
                self.playhead = t;
                self.draw();
                let due = begun + ((t - r.start + step) / 1000) as u64;
                let now = sex_pdx::get_ticks();
                if due > now {
                    let _ = sex_pdx::sys_wait_self_ticks(due - now);
                }
                t += step;
            }
            self.ed.cycle_audition();
            self.follow_caret();
        }

        fn write_doc(&mut self, filename: &str, content: &str) -> Result<(), &'static str> {
            let bytes = content.as_bytes();
            if files::size(&mut self.window, filename.as_bytes()).is_err() {
                files::create(&mut self.window, filename.as_bytes()).map_err(|_| "Cannot create handoff file.")?;
            }
            files::write(&mut self.window, filename.as_bytes(), bytes).map_err(|_| "Cannot write handoff file.")?;
            Ok(())
        }

        fn export_handoff(&mut self) {
            let stem = self.name.rsplit_once('.').map_or(self.name.as_str(), |(s, _)| s);
            let stem = String::from(stem.rsplit('/').next().unwrap_or(stem).trim_start_matches('@'));
            let bundle = sex_splice::handoff::build_handoff_bundle(self.ed.timeline(), Some(&self.ed.transcript), &stem);
            let write_all = (|| -> Result<(), &'static str> {
                self.write_doc(&format!("{stem}.fcpxml"), &bundle.fcpxml)?;
                self.write_doc(&format!("{stem}.otio"), &bundle.otio)?;
                self.write_doc(&format!("{stem}.edl"), &bundle.edl)?;
                if let Some(srt) = &bundle.srt {
                    self.write_doc(&format!("{stem}.srt"), srt)?;
                }
                self.write_doc(&format!("{stem}-handoff.md"), &bundle.readme)?;
                files::sync(&mut self.window).map_err(|_| "Cannot sync handoff bundle.")?;
                Ok(())
            })();
            match write_all {
                Ok(()) => {
                    sex_pdx::serial_record!("[splicy.handoff] stem={}", stem);
                    self.message = format!("Saved {stem} NLE handoff (.fcpxml, .otio, .edl, .srt, README)");
                }
                Err(why) => self.message = String::from(why),
            }
        }

        fn export(&mut self) {
            let stem = self.name.rsplit_once('.').map_or(self.name.as_str(), |(s, _)| s);
            // Into Splicy's own folder: a run may hold only the clip itself, not the folder it is in.
            let stem = stem.rsplit('/').next().unwrap_or(stem).trim_start_matches('@');
            let out_name = format!("{stem}-edit.webm");
            self.message = format!("Exporting {out_name}...");
            self.draw();
            let result = self.export_to(out_name.as_bytes());
            self.message = match result {
                Ok(frames) => {
                    let bytes = files::size(&mut self.window, out_name.as_bytes()).unwrap_or(0);
                    sex_pdx::serial_record!("[splicy.exported] file={} frames={} bytes={}", out_name, frames, bytes);
                    format!("Exported {out_name}: {frames} frames, {bytes} bytes")
                }
                Err(why) => {
                    sex_pdx::serial_record!("[splicy.export.failed] {}", why);
                    String::from(why)
                }
            };
        }

        fn export_to(&mut self, out: &[u8]) -> Result<u64, &'static str> {
            let tl = self.ed.timeline().clone();
            let Some(g) = self.grab.as_ref() else { return Err("Nothing to export.") };
            let (w, h) = (g.w & !1, g.h & !1);
            let config = Config { format: Format::WebmVp9Opus, width: w as u32, height: h as u32, fps_num: tl.fps_num, fps_den: tl.fps_den, sound: self.has_sound, quality: 0 };
            let mut ex = Exporter::new(config).map_err(|e| match e {
                sex_media::recording::Error::Unavailable => "This build has no video encoder (FFmpeg export profile).",
                _ => "Cannot start the encoder.",
            })?;
            match files::size(&mut self.window, out) {
                Ok(_) => files::write(&mut self.window, out, b"").map_err(|_| "Cannot replace the old export.")?,
                Err(_) => files::create(&mut self.window, out).map_err(|_| "Cannot create the export file.")?,
            }
            // The file's header is ready as soon as the encoder starts: write it before the
            // first picture, which the encoder takes only with nothing left to write.
            drain(&mut ex, &mut self.window, out)?;
            let wp: *mut Window = &mut self.window;
            let mut feed = if self.has_sound {
                let dec = open_decoder(&self.path, self.len).ok();
                Some(SoundFeed { dec, pieces: sound_pieces(&tl, RATE), piece: 0, written: 0, held: Vec::new(), held_at: 0, scratch: vec![[0; 2]; 8192] })
            } else {
                None
            };
            let total = (tl.duration() as i128 * RATE as i128 / SECOND as i128) as u64;
            let grab = self.grab.as_mut().ok_or("Nothing to export.")?;
            // The export pictures are the grabber's size, cropped to even.
            let mut src = Crop { g: grab, w, h };
            // Safety of the alias: the sink and the feed use the window only between the
            // grabber's reads, never during one, and all on this one thread.
            let window = unsafe { &mut *wp };
            let mut sink = Sink { ex: &mut ex, window, path: out, feed: feed.as_mut(), total, fps: (tl.fps_num, tl.fps_den), frames: 0 };
            let frames = export_pictures(&tl, None, w * h, &mut src, &mut sink).map_err(|e| match e {
                sex_splice::export::ExportError::Picture(_) => "Cannot read the clip's pictures.",
                sex_splice::export::ExportError::Output(why) => why,
                sex_splice::export::ExportError::Empty => "The edit is empty.",
            })?;
            if let Some(feed) = sink.feed.as_deref_mut() {
                feed.fill(sink.ex, sink.window, out, total)?;
            }
            ex.finish().map_err(|_| "The encoder could not finish the file.")?;
            let window = unsafe { &mut *wp };
            if !drain(&mut ex, window, out)? {
                return Err("The encoder did not finish.");
            }
            files::sync(window).map_err(|_| "Cannot save the export.")?;
            Ok(frames)
        }
    }

    /// The grabber's pictures cropped to even sizes for the encoder.
    struct Crop<'a> {
        g: &'a mut Grabber,
        w: usize,
        h: usize,
    }

    impl Pictures for Crop<'_> {
        type Error = sex_media::Error;
        fn picture(&mut self, _media: u32, source: Micros, out: &mut [u32]) -> Result<(), Self::Error> {
            self.g.show(source)?;
            for y in 0..self.h {
                out[y * self.w..(y + 1) * self.w].copy_from_slice(&self.g.picture[y * self.g.w..y * self.g.w + self.w]);
            }
            Ok(())
        }
    }

    trait CloseNow {
        fn close_now(&mut self) -> !;
    }
    impl CloseNow for Window {
        fn close_now(&mut self) -> ! {
            sex_pdx::serial_record!("[splicy.closed]");
            sex_pdx::task_finish(0);
        }
    }

    /// One pass over the clip for the timeline: a small picture every `every` ms and the
    /// loudness of all of its sound.
    fn scan(path: &[u8], len: u64, media: u32, width: u32, height: u32, duration_ms: u64) -> Library {
        let mut lib = Library::default();
        let Ok(mut dec) = open_decoder(path, len) else { return lib };
        let th = 54usize;
        let tw = if width > 0 && height > 0 { (th * width as usize / height as usize).clamp(16, 192) } else { 96 };
        let every = (duration_ms as i64 / 240).max(500);
        let mut pic = vec![0u32; tw * th];
        let mut pcm = vec![[0i16; 2]; 8192];
        let mut peaks = Peaks { per: 10_000, v: Vec::new() };
        let mut meter = sex_splice::edges::LevelMeter::new(20_000);
        let mut next_thumb = 0i64;
        let started = sex_pdx::get_ticks();
        loop {
            match dec.next(&mut pic, tw, th, &mut pcm) {
                Ok(Next::Picture { at_ms }) if at_ms >= next_thumb => {
                    lib.add_thumb(media, Thumb { at: at_ms.max(0) * 1000, w: tw, h: th, px: pic.clone() });
                    next_thumb = at_ms + every;
                }
                Ok(Next::Sound { frames, at_ms }) => {
                    let sound = &pcm[..frames.min(pcm.len())];
                    let at_us = at_ms.max(0) * 1000;
                    peaks.add(at_us, sound, RATE);
                    meter.add(at_us, sound, RATE);
                    lib.append_pcm(media, at_us, sound, RATE);
                }
                Ok(Next::End) | Err(_) => break,
                _ => {}
            }
        }
        sex_pdx::serial_record!(
            "[splicy.scan] thumbs={} peaks={} loudest={} ms={} pcm_frames={}",
            lib.thumbs.get(&media).map_or(0, |t| t.len()),
            peaks.v.len(),
            peaks.v.iter().max().copied().unwrap_or(0),
            sex_pdx::get_ticks() - started,
            lib.pcm.get(&media).map_or(0, |p| p.len())
        );
        lib.peaks.insert(media, peaks);
        lib.levels.insert(media, meter.levels());
        lib
    }

    fn say(window: &mut Window, text: &str) {
        sex_pdx::serial_record!("[splicy.message] {}", text);
        window.show(text.as_bytes());
    }

    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        let mut window = Window::open();
        window.title(b"Splicy");
        let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
        let given = files::args(&mut window, &mut args).to_vec();
        let mut tokens = given.split(|b| *b == b' ').filter(|w| !w.is_empty());
        let path: Vec<u8> = tokens.next().unwrap_or(b"").to_vec();
        // A run holds only the files it was given, so the transcript may come as its own
        // argument (`run splicy @talk.webm @talk.words`); otherwise it is looked for beside the clip.
        let words_arg: Option<String> = tokens.next().map(|w| String::from_utf8_lossy(w).into_owned());
        if path.is_empty() {
            say(&mut window, "Splicy edits a clip by editing its transcript.\n\nUsage: run splicy @talk.webm @talk.words");
            loop {
                if let Event::Close = window.next() {
                    window.close()
                }
            }
        }
        let name = String::from_utf8_lossy(&path).into_owned();
        let len = match files::size(&mut window, &path) {
            Ok(n) => n,
            Err(e) => {
                say(&mut window, core::str::from_utf8(files::message(e)).unwrap_or("Cannot open that file."));
                loop {
                    if let Event::Close = window.next() {
                        window.close()
                    }
                }
            }
        };
        // The transcript beside the clip.
        let words_path = words_arg.unwrap_or_else(|| words::beside(&name));
        let mut text = vec![0u8; 1 << 20];
        let transcript = match files::read(&mut window, words_path.as_bytes(), &mut text) {
            Ok(n) => match words::parse(&String::from_utf8_lossy(&text[..n])) {
                Ok(t) => t,
                Err(words::BadLine(line)) => {
                    sex_pdx::serial_record!("[splicy.words.bad] line={}", line);
                    sex_splice::transcript::Transcript { media: 0, words: Vec::new() }
                }
            },
            Err(_) => sex_splice::transcript::Transcript { media: 0, words: Vec::new() },
        };
        drop(text);
        let (grab, duration, fps, has_sound, dims) = match open_decoder(&path, len) {
            Ok(dec) => {
                let info = dec.info.clone();
                let (w, h) = sex_media::fit(info.width.max(2) as usize, info.height.max(2) as usize, 640, 360);
                let fps = if info.fps.0 == 0 || info.fps.1 == 0 { (30, 1) } else { info.fps };
                let frame_ms = (1000 * fps.1 as i64 / fps.0 as i64).max(1);
                sex_pdx::serial_record!(
                    "[splicy.opened] file={} format={} video={} audio={} w={} h={} fps={}/{} ms={} words={}",
                    name, info.format, info.video_codec, info.audio_codec, info.width, info.height, fps.0, fps.1, info.duration_ms, transcript.words.len()
                );
                let g = if info.width > 0 {
                    Some(Grabber { dec, w, h, picture: vec![0; w * h], at_ms: -1, frame_ms, pcm: vec![[0; 2]; 8192], cache: FrameCache::new(CacheConfig::low_memory()) })
                } else {
                    None
                };
                (g, info.duration_ms as i64 * 1000, fps, info.sample_rate > 0, (info.width, info.height))
            }
            Err(e) => {
                let mut buf = [0u8; 96];
                sex_pdx::serial_record!("[splicy.open.failed] {}", e.text(&mut buf));
                // Edit the transcript alone: the clip's length is where its last word ends.
                let end = transcript.words.last().map_or(SECOND, |w| w.source.end + SECOND);
                (None, end, (30, 1), true, (0, 0))
            }
        };
        let ed = Editor::open(&name, duration, fps, has_sound, transcript);
        say(&mut window, "Reading the clip for the timeline...");
        let lib = scan(&path, len, ed.transcript.media, dims.0, dims.1, (duration / 1000) as u64);
        let audio = if has_sound {
            LiveAudio::open(ed.timeline(), ed.history.head())
        } else {
            None
        };
        let now_tick = sex_pdx::get_ticks();
        let mut app = App {
            window,
            path,
            len,
            name,
            ed,
            view: View::default(),
            grab,
            audio,
            lib,
            has_sound,
            playhead: 0,
            last_tick_ms: now_tick,
            next_frame_tick: now_tick,
            dragging_timeline: false,
            dragging_text: false,
            message: String::new(),
        };
        app.report();
        app.draw();
        loop {
            let ev = if app.ed.shuttle_rate != 0 {
                let frame_ms = (app.frame_step_us() / 1000).clamp(16, 40) as u64;
                let now = sex_pdx::get_ticks();
                let wait_ms = app.next_frame_tick.saturating_sub(now);
                let polled = if wait_ms == 0 {
                    app.window.poll()
                } else {
                    app.window.next_within(wait_ms)
                };
                match polled {
                    Some(ev) => ev,
                    None => {
                        let after = sex_pdx::get_ticks();
                        app.next_frame_tick = (app.next_frame_tick + frame_ms).max(after + 1);
                        app.step_shuttle();
                        app.draw();
                        continue;
                    }
                }
            } else {
                app.window.next()
            };
            match ev {
                Event::Close => app.window.close_now(),
                Event::Resize { .. } => {}
                Event::Key(key) => app.key(key),
                Event::Pointer(p) => match p.kind {
                    Kind::Down { button: 0 } => {
                        app.dragging_timeline = false;
                        app.dragging_text = false;
                        let (w, h) = app.window.size();
                        let (wi, hi) = (w as i32, h as i32);
                        let (x, y) = (p.x as i32, p.y as i32);
                        if let Some(hit) = app.view.toolbar_hit(wi, hi, x, y) {
                            match hit {
                                ToolbarHit::Layout(mode) => {
                                    app.view.layout = mode;
                                    app.message = match mode {
                                        splicy::LayoutMode::FinalCut => "Layout: Final Cut Pro".into(),
                                        splicy::LayoutMode::Premiere => "Layout: Premiere".into(),
                                    };
                                }
                                ToolbarHit::Tool(tool) => {
                                    app.ed.active_tool = tool;
                                    app.message = format!("{} tool", tool.label());
                                }
                                ToolbarHit::ToggleVu => {
                                    app.view.vu.visible = !app.view.vu.visible;
                                }
                                ToolbarHit::ToggleInspector => {
                                    app.view.show_inspector = !app.view.show_inspector;
                                }
                            }
                        } else if let Some(th) = app.view.transport_hit(wi, hi, x, y) {
                            match th {
                                TransportHit::PlayPause => app.toggle_play_pause(),
                                TransportHit::Start => {
                                    app.stop_playback();
                                    app.playhead = app.ed.in_point.unwrap_or(0);
                                    if let Some(i) = app.ed.word_at_time(app.playhead) {
                                        app.ed.place(i, false);
                                    }
                                    app.message = "Jumped to start".into();
                                }
                                TransportHit::End => {
                                    app.stop_playback();
                                    let dur = app.ed.timeline().duration();
                                    app.playhead = app.ed.out_point.unwrap_or(dur);
                                    if let Some(i) = app.ed.word_at_time(app.playhead) {
                                        app.ed.place(i, false);
                                    }
                                    app.message = "Jumped to end".into();
                                }
                                TransportHit::StepBack => app.step_frames(-1),
                                TransportHit::StepFwd => app.step_frames(1),
                                TransportHit::CleanupOrAudition => {
                                    if !app.ed.joins().is_empty() {
                                        app.stop_playback();
                                        app.preview_join();
                                    } else {
                                        app.run_cleanup();
                                    }
                                }
                            }
                        } else if app.view.player_hit(wi, hi, x, y) {
                            app.toggle_play_pause();
                        } else if let Some(ch) = app.view.card_hit(&app.ed, wi, hi, x, y) {
                            match ch {
                                CardHit::RunCleanup => app.run_cleanup(),
                                CardHit::Select(idx) => {
                                    app.ed.card = idx;
                                    if let Some(&id) = app.ed.pending().get(idx) {
                                        if let Some(s) = app.ed.suggestions.get(id) {
                                            if let Some(cut) = s.cuts.first() {
                                                if let Some(t) = app
                                                    .ed
                                                    .timeline()
                                                    .tracks
                                                    .iter()
                                                    .flat_map(|tr| tr.clips.iter())
                                                    .find_map(|cl| cl.timeline_at(cut.range.start))
                                                {
                                                    app.stop_playback();
                                                    app.playhead = t;
                                                }
                                            }
                                        }
                                    }
                                }
                                CardHit::Accept(idx) => {
                                    app.ed.card = idx;
                                    match app.ed.accept() {
                                        Ok(true) => {
                                            sex_pdx::serial_record!("[splicy.suggestion] taken=1");
                                            app.report();
                                        }
                                        Ok(false) => {}
                                        Err(_) => app.message = "That suggestion no longer fits the edit.".into(),
                                    }
                                }
                                CardHit::Skip(idx) => {
                                    app.ed.card = idx;
                                    if app.ed.skip() {
                                        sex_pdx::serial_record!("[splicy.suggestion] skipped=1");
                                    }
                                }
                            }
                        } else if let Some(sw) = app.view.track_switch_hit(app.ed.timeline(), wi, hi, x, y) {
                            match sw {
                                TrackSwitchHit::ToggleMute(ti) => {
                                    let muted = app.ed.toggle_track_muted(ti);
                                    app.message = if muted { "Audio track muted".into() } else { "Audio track unmuted".into() };
                                    app.report();
                                }
                                TrackSwitchHit::ToggleVideo(ti) => {
                                    if let Ok(vis) = app.ed.toggle_track_enabled(ti) {
                                        app.message = if vis { "Video track shown".into() } else { "Video track hidden".into() };
                                        app.report();
                                    }
                                }
                            }
                        } else if let Some(i) = app.view.word_at(&app.ed, wi, hi, x, y) {
                            app.stop_playback();
                            app.ed.place(i, false);
                            app.follow_caret();
                            app.dragging_text = true;
                        } else if let Some(t) = app.view.time_at(app.ed.timeline(), wi, hi, x, y) {
                            app.stop_playback();
                            app.playhead = t;
                            if let Some(i) = app.ed.word_at_time(t) {
                                app.ed.place(i, false);
                            }
                            if app.ed.active_tool == ToolKind::Blade && app.view.areas(wi, hi).tracks.contains(x, y) {
                                if let Ok(true) = app.ed.blade_at(t) {
                                    app.message = "Blade cut".into();
                                    app.report();
                                }
                            } else {
                                app.dragging_timeline = true;
                            }
                        }
                    }
                    Kind::Move => {
                        if app.dragging_timeline {
                            let (w, h) = app.window.size();
                            let t = app.view.scrub_time_at_x(app.ed.timeline(), w as i32, h as i32, p.x as i32);
                            app.playhead = t;
                            if let Some(i) = app.ed.word_at_time(t) {
                                app.ed.place(i, false);
                            }
                        } else if app.dragging_text {
                            let (w, h) = app.window.size();
                            if let Some(i) = app.view.word_at(&app.ed, w as i32, h as i32, p.x as i32, p.y as i32) {
                                app.ed.place(i, true);
                            }
                        } else {
                            continue;
                        }
                    }
                    Kind::Up { .. } | Kind::Leave => {
                        app.dragging_timeline = false;
                        app.dragging_text = false;
                        continue;
                    }
                    Kind::Wheel { steps } => {
                        let (w, h) = app.window.size();
                        if app.view.time_at(app.ed.timeline(), w as i32, h as i32, p.x as i32, p.y as i32).is_some() {
                            app.view.zoom_by(app.ed.timeline(), w as i32, steps as i32, app.playhead);
                        } else {
                            app.view.scroll = (app.view.scroll - steps as i32 * 40).max(0);
                        }
                    }
                    _ => continue,
                },
                _ => continue,
            }
            app.draw();
        }
    }
}
