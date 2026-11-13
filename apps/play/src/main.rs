// SPDX-License-Identifier: GPL-3.0-only
//! Play: plays video and sound. `run play @clip.mp4`. Decoding is sex-media's
//! (FFmpeg inside this program's own confinement): this program reads the file
//! through Files, shows each picture in its window and handles keys. A hostile file
//! can only crash this window.
//!
//! Keys: Space or K pauses and plays; Left/J and Right/L go back and forward 5 s;
//! Home goes to the start. Subtitles come from a SubRip or SubStation file given after
//! the video (`run play @film.mp4 @film.srt`), or one beside it when Play may read it.
//! S shows or hides subtitles, A plays the next sound track,
//! [ and ] play slower and faster (= back to normal). F11 closes. The keyboard's media keys work too, wherever
//! the focus is, while Play is the program playing (Shell routes them): play/pause
//! toggles and stop pauses at the start.
#![no_std]
#![no_main]

extern crate alloc;
mod captions;
mod encode_proof;

use alloc::boxed::Box;
use alloc::vec::Vec;
use sex_app::{files, Event, Window};
use sex_media::{Decoder, Player, Source, Step};
use sex_pdx::pixel_region::{MAX_H, MAX_W};

/// The file, read through Files a piece at a time, so its size is not limited by
/// memory.
struct FileSource {
    window: *mut Window,
    path: Vec<u8>,
    len: u64,
    cache: Vec<u8>,
    cache_at: u64,
}

const CACHE: usize = 256 * 1024;

impl Source for FileSource {
    fn len(&self) -> Option<u64> { Some(self.len) }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        if at >= self.len {
            return Some(0);
        }
        let in_cache = at >= self.cache_at && at < self.cache_at + self.cache.len() as u64;
        if !in_cache {
            let want = (CACHE as u64).min(self.len - at) as usize;
            self.cache.resize(want, 0);
            // The window outlives the player: both live until the program ends.
            let window = unsafe { &mut *self.window };
            match files::read_at(window, &self.path, at, &mut self.cache) {
                Ok(got) if got > 0 => { self.cache.truncate(got); self.cache_at = at; }
                _ => return None,
            }
        }
        let off = (at - self.cache_at) as usize;
        let n = out.len().min(self.cache.len() - off);
        out[..n].copy_from_slice(&self.cache[off..off + n]);
        Some(n)
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[play.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn say(window: &mut Window, why: &[u8]) {
    let mut text = [0u8; 300];
    let head = b"Play. Space pauses, J/L or Left/Right seek, S subtitles, A sound track, [ ] speed, F11 closes.\n\n";
    let n = (head.len() + why.len()).min(text.len());
    text[..head.len()].copy_from_slice(head);
    text[head.len()..n].copy_from_slice(&why[..n - head.len()]);
    window.show(&text[..n]);
}

fn fail(window: &mut Window, why: &[u8]) {
    sex_pdx::serial_record!("[play.failed] {}", core::str::from_utf8(why).unwrap_or("?"));
    say(window, why);
}

const SEEK_MS: i64 = 5000;

/// Subtitles, while shown.
struct Subs { track: sex_media::subtitles::Track, painter: captions::Painter, on: bool, text: alloc::string::String }

/// How one item of the playlist ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome { Ended, Next, Previous, Closed }

/// What is drawn over the picture besides the picture: subtitles, and the controls
/// (state, time, place in the playlist, speed) for a moment after each key and while
/// paused.
struct Ui {
    subs: Option<Subs>,
    controls: Option<captions::Painter>,
    controls_until: u64,
    /// This item and how many there are.
    item: (usize, usize),
    /// The time shown last, so a paused picture is drawn again only when it changes.
    drawn_ms: i64,
}

const CONTROLS_MS: u64 = 2500;

fn clock(out: &mut alloc::string::String, ms: u64) {
    use core::fmt::Write;
    let s = ms / 1000;
    if s >= 3600 { let _ = write!(out, "{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60); }
    else { let _ = write!(out, "{}:{:02}", s / 60, s % 60); }
}

/// The current picture with subtitles and controls over it.
fn present(window: &mut Window, player: &mut Player, ui: &mut Ui, at_ms: i64) {
    let (w, h) = player.size();
    if w == 0 || player.picture().len() < w * h { return; }
    window.pixels()[..w * h].copy_from_slice(player.picture());
    if let Some(s) = ui.subs.as_mut().filter(|s| s.on) {
        let mut text = core::mem::take(&mut s.text);
        s.track.at(at_ms.max(0) as u64, &mut text);
        if text != s.painter.shown {
            sex_pdx::serial_record!("[play.caption] at_ms={} text={}", at_ms, text.replace('\n', " / "));
            s.painter.shown.clone_from(&text);
        }
        s.painter.draw(&mut window.pixels()[..w * h], w, h, &text);
        s.text = text;
    }
    let showing = player.paused() || sex_pdx::get_ticks() < ui.controls_until;
    if let (Some(c), true) = (ui.controls.as_mut(), showing) {
        use core::fmt::Write;
        let mut text = alloc::string::String::new();
        text.push_str(if player.paused() { "Paused  " } else { "Playing  " });
        clock(&mut text, at_ms.max(0) as u64);
        let total = player.duration_ms();
        if total > 0 { text.push_str(" / "); clock(&mut text, total); }
        if ui.item.1 > 1 { let _ = write!(text, "   {} of {}", ui.item.0 + 1, ui.item.1); }
        if player.speed() != 100 { let _ = write!(text, "   {}%", player.speed()); }
        let progress = if total > 0 { (at_ms.max(0) as u64).min(total) * 1000 / total } else { 0 };
        c.controls(&mut window.pixels()[..w * h], w, h, &text, progress as u32);
        if text != c.shown {
            sex_pdx::serial_record!("[play.controls] text={}", text);
            c.shown = text;
        }
    }
    ui.drawn_ms = at_ms;
    window.show_pixels(w, h);
}

/// Handles keys and close; an outcome when this item is done.
fn keys(window: &mut Window, player: &mut Player, ui: &mut Ui) -> Option<Outcome> {
    let mut any = false;
    while let Some(e) = window.poll() {
        match e {
            Event::Close => return Some(Outcome::Closed),
            Event::Key(k) => {
                any = true;
                match k.code {
                57 | 37 | sex_pdx::app_key::KEY_PLAY_PAUSE => {
                    if player.paused() { player.play() } else { player.pause() }
                    sex_pdx::serial_record!("[play.paused] {}", player.paused() as u8);
                }
                0x69 | 0x6a | 36 | 38 => {
                    let by = if k.code == 0x6a || k.code == 38 { SEEK_MS } else { -SEEK_MS };
                    let to = (player.time_ms() + by).max(0) as u64;
                    sex_pdx::serial_record!("[play.seek] to_ms={}", to);
                    player.seek(to);
                }
                0x66 => player.seek(0),
                // N and the next key: the next item; P and the previous key: back to
                // this item's start, or the item before when near its start.
                0x31 | sex_pdx::app_key::KEY_NEXT => return Some(Outcome::Next),
                0x19 | sex_pdx::app_key::KEY_PREVIOUS => {
                    if player.time_ms() < 3000 && ui.item.0 > 0 { return Some(Outcome::Previous); }
                    player.seek(0);
                }
                // S: subtitles on or off.
                0x1F => if let Some(s) = ui.subs.as_mut() {
                    s.on = !s.on;
                    sex_pdx::serial_record!("[play.subtitles] on={}", s.on as u8);
                },
                // A: the next sound track.
                0x1E => {
                    let (n, now) = player.audio_tracks();
                    if n > 1 {
                        let next = now.map_or(0, |c| (c + 1) % n);
                        let ok = player.set_audio_track(next).is_ok();
                        sex_pdx::serial_record!("[play.audio.track] track={} of={} ok={}", next, n, ok as u8);
                    } else {
                        sex_pdx::serial_record!("[play.audio.track] track={} of={} ok=0", now.map_or(-1, |c| c as i64), n);
                    }
                }
                // [ slower, ] faster, = normal: quarter steps from 50% to 200%.
                0x1A | 0x1B | 0x0D => {
                    let now = player.speed();
                    let to = match k.code { 0x1A => now.saturating_sub(25).max(50), 0x1B => (now + 25).min(200), _ => 100 };
                    player.set_speed(to);
                    sex_pdx::serial_record!("[play.speed] percent={}", player.speed());
                }
                sex_pdx::app_key::KEY_STOP => {
                    player.pause();
                    player.seek(0);
                    sex_pdx::serial_record!("[play.stopped] paused={}", player.paused() as u8);
                }
                _ => {}
                }
            }
            _ => {}
        }
    }
    if any {
        ui.controls_until = sex_pdx::get_ticks() + CONTROLS_MS;
        let at = player.time_ms();
        present(window, player, ui, at);
    }
    None
}

struct Played { pictures: u32, sound_frames: u64, outcome: Outcome, at_ms: i64, duration_ms: u64 }

/// Plays the stream from `start_ms`; returns what was shown and heard and how it
/// ended, or FFmpeg's error. `progress` is called now and then with the time, to save it.
fn play(window: &mut Window, src: FileSource, ui: &mut Ui, start_ms: u64, progress: &mut dyn FnMut(&mut Window, u64, u64)) -> Result<Played, sex_media::Error> {
    let dec = Decoder::open(Box::new(src))?;
    let info = dec.info.clone();
    let (w, h) = sex_media::fit(info.width as usize, info.height as usize, MAX_W, MAX_H);
    let (num, den) = match info.fps { (0, _) | (_, 0) => (25, 1), f => f };
    sex_pdx::serial_record!(
        "[play.opened] format={} codec={} w={} h={} fps={}/{} ms={} shown={}x{} audio={} rate={} channels={}",
        info.format, info.video_codec, info.width, info.height, num, den, info.duration_ms, w, h,
        info.audio_codec, info.sample_rate, info.channels
    );
    let begun = sex_pdx::get_ticks();
    let mut player = Player::new(dec, w, h, true, false);
    if info.sample_rate > 0 && !player.has_sound() {
        sex_pdx::serial_record!("[play.sound.none] reason=no_ring");
    }
    if start_ms > 0 {
        sex_pdx::serial_record!("[play.resume] ms={}", start_ms);
        player.seek(start_ms);
    }
    ui.controls_until = sex_pdx::get_ticks() + CONTROLS_MS;
    let (w, h) = player.size();
    if w == 0 {
        let mut text = Vec::new();
        text.extend_from_slice(b"Playing sound: ");
        text.extend_from_slice(info.audio_codec.as_str().as_bytes());
        text.extend_from_slice(b" in ");
        text.extend_from_slice(info.format.as_str().as_bytes());
        say(window, &text);
    }
    let mut stalled = false;
    let mut saved_at = sex_pdx::get_ticks();
    let mut flash = false;
    let outcome = loop {
        if let Some(o) = keys(window, &mut player, ui) { break o; }
        if let (Some(sex_media::Silence::Stalled { played, pending }), false) = (player.silence(), stalled) {
            stalled = true;
            sex_pdx::serial_record!("[play.sound.stalled] played={} pending={}", played, pending);
        }
        if sex_pdx::get_ticks() - saved_at > 5000 {
            saved_at = sex_pdx::get_ticks();
            let at = player.time_ms().max(0) as u64;
            progress(window, at, player.duration_ms());
        }
        match player.step() {
            Step::Picture { n, at_ms } => {
                present(window, &mut player, ui, at_ms);
                // A picture that turns bright in the middle is logged with the tick it
                // was shown at: the A/V sync gate lines these up with its beeps.
                let mid = player.picture().get((h / 2) * w + w / 2).copied().unwrap_or(0);
                let bright = (mid >> 16 & 0xFF) + (mid >> 8 & 0xFF) + (mid & 0xFF) > 600;
                if bright && !flash { sex_pdx::serial_record!("[play.flash] at_ms={} tick={} played={}", at_ms, sex_pdx::get_ticks(), player.played()); }
                flash = bright;
                if n < 3 || n % 10 == 0 {
                    let px = player.picture();
                    let hash = px.iter().fold(0xcbf2_9ce4_8422_2325u64, |a, &p| (a ^ p as u64).wrapping_mul(0x100_0000_01b3));
                    sex_pdx::serial_record!("[play.frame] n={} at_ms={} hash={:016x} probe={:06x}", n, at_ms, hash, px[(h / 2) * w + 3 * w / 4] & 0xFF_FFFF);
                }
            }
            Step::Wait { ms } => { let _ = sex_pdx::sys_wait_self_ticks(ms.clamp(1, 16)); }
            Step::Paused => {
                // Controls fade a moment after the last key even while paused.
                let _ = sex_pdx::sys_wait_self_ticks(30);
            }
            Step::Ended => break Outcome::Ended,
        }
    };
    let at_ms = player.time_ms();
    if outcome == Outcome::Closed {
        sex_pdx::serial_record!("[play.closed] target={} task={} frames={}", window.target, window.task, player.shown);
    }
    if let Some(e) = player.error() {
        return Err(e);
    }
    let ms = sex_pdx::get_ticks() - begun;
    if outcome == Outcome::Ended {
        sex_pdx::serial_record!("[play.done] frames={} ms={} w={} h={} sound_frames={} played={} late_ms={}",
            player.shown, ms, w, h, player.heard, player.played(), player.late_ms);
    }
    Ok(Played { pictures: player.shown, sound_frames: player.heard, outcome, at_ms, duration_ms: player.duration_ms() })
}

/// Where each video was left, in Play's own folder: "ms<TAB>path" lines, newest first.
const RESUME_FILE: &[u8] = b"play-resume.txt";
const RESUME_MAX: usize = 64;
/// Positions this close to either end are not kept: start again from the beginning.
const RESUME_EDGE_MS: u64 = 3000;

struct Resume { entries: Vec<(u64, Vec<u8>)> }
impl Resume {
    fn load(window: &mut Window) -> Resume {
        let mut buf = alloc::vec![0u8; 16 * 1024];
        let n = files::read(window, RESUME_FILE, &mut buf).unwrap_or(0);
        let entries = buf[..n].split(|b| *b == b'\n').filter_map(|l| {
            let tab = l.iter().position(|b| *b == b'\t')?;
            let ms = core::str::from_utf8(&l[..tab]).ok()?.parse().ok()?;
            Some((ms, l[tab + 1..].to_vec()))
        }).take(RESUME_MAX).collect();
        Resume { entries }
    }
    fn get(&self, path: &[u8]) -> u64 { self.entries.iter().find(|e| e.1 == path).map_or(0, |e| e.0) }
    /// Remembers `ms` for `path`, or forgets it near either end. Writes when it changed.
    fn set(&mut self, window: &mut Window, path: &[u8], ms: u64, duration: u64) {
        let keep = ms >= RESUME_EDGE_MS && (duration == 0 || ms + RESUME_EDGE_MS < duration);
        let before = self.get(path);
        if (keep && before == ms) || (!keep && before == 0) { return; }
        self.entries.retain(|e| e.1 != path);
        if keep { self.entries.insert(0, (ms, path.to_vec())); self.entries.truncate(RESUME_MAX); }
        let mut out = Vec::new();
        for (ms, p) in &self.entries {
            out.extend_from_slice(alloc::format!("{}\t", ms).as_bytes());
            out.extend_from_slice(p);
            out.push(b'\n');
        }
        let ok = files::write(window, RESUME_FILE, &out).is_ok();
        sex_pdx::serial_record!("[play.resume.saved] ms={} kept={} ok={}", ms, keep as u8, ok as u8);
    }
}

fn is_subtitles(w: &[u8]) -> bool {
    [&b".srt"[..], b".ass", b".ssa"].iter().any(|e| w.len() > e.len() && w[w.len() - e.len()..].eq_ignore_ascii_case(e))
}

/// Plays the playlist: each video, with the subtitle file named right after it.
fn playlist(window: &mut Window, words: &[&[u8]]) {
    let mut items: Vec<(&[u8], Option<&[u8]>)> = Vec::new();
    for w in words.iter().copied().filter(|w| !w.starts_with(b"--")) {
        match items.last_mut() {
            Some(last) if is_subtitles(w) && last.1.is_none() => last.1 = Some(w),
            _ => items.push((w, None)),
        }
    }
    if items.is_empty() { return fail(window, b"Usage: run play @VIDEO.mp4 [@SUBTITLES.srt] [@MORE.mp4 ...]"); }
    let mut resume = Resume::load(window);
    let mut ui = Ui { subs: None, controls: captions::Painter::new(), controls_until: 0, item: (0, items.len()), drawn_ms: 0 };
    let mut i = 0usize;
    while i < items.len() {
        let (path, given) = items[i];
        ui.item = (i, items.len());
        sex_pdx::serial_record!("[play.item] n={} of={} file={}", i, items.len(), core::str::from_utf8(path).unwrap_or("?"));
        let len = match files::size(window, path) {
            Ok(len) => len,
            Err(e) => {
                fail(window, files::message(e));
                i += 1;
                continue;
            }
        };
        say(window, b"Opening...");
        ui.subs = captions::load(window, path, given).and_then(|(file, track)| {
            sex_pdx::serial_record!("[play.subtitles.loaded] file={} cues={}", core::str::from_utf8(&file).unwrap_or("?"), track.cues.len());
            Some(Subs { track, painter: captions::Painter::new()?, on: true, text: alloc::string::String::new() })
        });
        let start = resume.get(path);
        let wp: *mut Window = window;
        let src = FileSource { window: wp, path: path.to_vec(), len, cache: Vec::new(), cache_at: 0 };
        let rp: *mut Resume = &mut resume;
        let mut save = |w: &mut Window, at: u64, total: u64| unsafe { (*rp).set(w, path, at, total) };
        match play(unsafe { &mut *wp }, src, &mut ui, start, &mut save) {
            Ok(p) => {
                // Where it was left; an item played to its end starts afresh next time.
                let at = if p.outcome == Outcome::Ended { 0 } else { p.at_ms.max(0) as u64 };
                resume.set(window, path, at, p.duration_ms);
                match p.outcome {
                    Outcome::Closed => sex_pdx::task_finish(0),
                    Outcome::Previous => { i = i.saturating_sub(1); continue; }
                    Outcome::Next | Outcome::Ended => {}
                }
                if p.pictures == 0 && p.sound_frames == 0 { fail(window, b"That file has nothing to play."); }
                else if p.pictures == 0 && i + 1 == items.len() { say(window, b"Finished playing."); }
            }
            Err(e) => {
                let mut text = [0u8; 96];
                let mut why = Vec::new();
                why.extend_from_slice(b"Cannot play that file: ");
                why.extend_from_slice(e.text(&mut text).as_bytes());
                fail(window, &why);
            }
        }
        i += 1;
    }
    sex_pdx::serial_record!("[play.playlist.done] items={}", items.len());
}

/// cos and sin of `x` (|x| up to about 4), by their series, since programs have no libm here.
fn cos_sin(x: f64) -> (f64, f64) {
    let (mut c, mut s, mut term_c, mut term_s) = (0.0, 0.0, 1.0, x);
    for k in 0..20 {
        c += term_c; s += term_s;
        let n = (2 * k + 1) as f64;
        term_c *= -x * x / (n * (n + 1.0));
        term_s *= -x * x / ((n + 1.0) * (n + 2.0));
    }
    (c, s)
}

/// `play --tone HZ RATE`: two seconds of a sine at HZ, written at RATE frames a second,
/// for checking that Bell converts every program rate to its own.
fn tone(window: &mut Window, hz: u32, rate: u32) {
    let Some(mut sound) = sex_app::sound::Sound::open() else { return fail(window, b"No sound for this program.") };
    sound.set_rate(rate);
    let (cw, sw) = cos_sin(2.0 * core::f64::consts::PI * hz as f64 / rate as f64);
    let (mut c, mut s) = (1.0f64, 0.0f64);
    let total = 2 * rate as u64;
    let mut made = 0u64;
    let mut buf = [[0i16; 2]; 1024];
    let (mut at, mut len) = (0usize, 0usize);
    while made < total || at < len {
        if at == len {
            let n = (total - made).min(buf.len() as u64) as usize;
            for f in buf.iter_mut().take(n) {
                let v = (s * 12000.0) as i16;
                *f = [v, v];
                (c, s) = (c * cw - s * sw, s * cw + c * sw);
            }
            // Keep the oscillator on the unit circle.
            let fix = 1.5 - 0.5 * (c * c + s * s);
            (c, s) = (c * fix, s * fix);
            made += n as u64;
            (at, len) = (0, n);
        }
        let took = sound.write(&buf[at..len]);
        at += took;
        if took == 0 { let _ = sex_pdx::sys_wait_self_ticks(5); }
    }
    sound.close();
    while sound.pending() > 0 { let _ = sex_pdx::sys_wait_self_ticks(10); }
    sex_pdx::serial_record!("[play.tone] hz={} rate={} frames={} played={}", hz, rate, total, sound.played());
    say(window, b"Tone played.");
}

/// `play --listen SECONDS`: asks for the microphone, waits for the person's leave, then
/// counts what it hears for SECONDS and reports frames, loudest sample and losses.
fn listen(window: &mut Window, seconds: u32) {
    use sex_pdx::sound_ring as ring;
    let Some(mut sound) = sex_app::sound::Sound::open() else { return fail(window, b"No sound for this program.") };
    let row = ring::index_of(window.target).map_or(-1, |r| r as i64);
    sound.listen(true);
    sex_pdx::serial_record!("[play.listen.asked] row={}", row);
    say(window, b"Asking for the microphone. Allow it with: volume ROW mic on");
    let mut last = u64::MAX;
    let begun = sex_pdx::get_ticks();
    loop {
        let mic = sound.mic();
        if mic != last { sex_pdx::serial_record!("[play.listen.state] row={} mic={}", row, mic); last = mic; }
        if mic == ring::MIC_ON { break; }
        if mic == ring::MIC_NONE { return fail(window, b"There is no microphone."); }
        if sex_pdx::get_ticks() - begun > 120_000 { return fail(window, b"The microphone was not allowed."); }
        if let Some(Event::Close) = window.poll() { sound.listen(false); return; }
        let _ = sex_pdx::sys_wait_self_ticks(20);
    }
    say(window, b"Listening.");
    let (mut frames, mut peak, mut buf) = (0u64, 0i32, [[0i16; 2]; 1024]);
    let start = sex_pdx::get_ticks();
    // Frames before the first period arrive are not counted: time runs from it.
    let mut first = None;
    while first.map_or(true, |t| sex_pdx::get_ticks() - t < seconds as u64 * 1000) {
        let n = sound.hear(&mut buf);
        if n > 0 && first.is_none() { first = Some(sex_pdx::get_ticks()); }
        frames += n as u64;
        peak = buf[..n].iter().fold(peak, |p, f| p.max((f[0] as i32).abs()).max((f[1] as i32).abs()));
        if sound.mic() != ring::MIC_ON { break; }
        if first.is_none() && sex_pdx::get_ticks() - start > 10_000 { break; }
        if n == 0 { let _ = sex_pdx::sys_wait_self_ticks(10); }
    }
    let ms = first.map_or(0, |t| sex_pdx::get_ticks() - t);
    sex_pdx::serial_record!("[play.listen] row={} frames={} ms={} peak={} lost={} mic={}", row, frames, ms, peak, sound.lost(), sound.mic());
    // Stay, still asking, until the person takes the leave back or closes the window.
    say(window, b"Heard the microphone.");
    while sound.mic() == ring::MIC_ON {
        if let Some(Event::Close) = window.poll() { break; }
        let mut buf = [[0i16; 2]; 1024];
        sound.hear(&mut buf);
        let _ = sex_pdx::sys_wait_self_ticks(20);
    }
    sex_pdx::serial_record!("[play.listen.end] row={} mic={}", row, sound.mic());
    sound.listen(false);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Play");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let words: Vec<&[u8]> = given.split(|b| *b == b' ').filter(|w| !w.is_empty()).collect();
    let number = |w: Option<&&[u8]>| w.and_then(|w| core::str::from_utf8(w).ok()).and_then(|t| t.parse::<u32>().ok());
    let file = if words.first() == Some(&&b"--encode-proof"[..]) {
        match encode_proof::run(&mut window, words.get(1).copied().unwrap_or(b"")) {
            Ok(()) => say(&mut window, b"Synthetic encoder proof saved. This is not a desktop recording."),
            Err(why) => fail(&mut window, why),
        }
        None
    } else if words.first() == Some(&&b"--listen"[..]) {
        listen(&mut window, number(words.get(1)).unwrap_or(2));
        None
    } else if words.first() == Some(&&b"--tone"[..]) {
        tone(&mut window, number(words.get(1)).unwrap_or(440), number(words.get(2)).unwrap_or(sex_app::sound::RATE));
        None
    } else { words.first().map(|f| f.to_vec()) };
    if file.is_some() { playlist(&mut window, &words); }
    else if !words.first().is_some_and(|w| w.starts_with(b"--")) { fail(&mut window, b"Usage: run play @VIDEO.mp4 [@SUBTITLES.srt] [@MORE.mp4 ...]"); }
    sex_pdx::serial_record!("[play.ready] target={} task={}", window.target, window.task);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[play.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
