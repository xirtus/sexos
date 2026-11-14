// SPDX-License-Identifier: GPL-3.0-only
//! Music: a music library and player. `run music @/Music` (a folder, read through
//! Files, its subfolders too) or `run music @a.flac @b.mp3`. Tracks are grouped by
//! artist and album from their tags (FFmpeg reads them inside this program's
//! confinement) and play one after another with no gap between them: the next track
//! is opened while the current one plays, and its first sound follows the last sound
//! of the one before in the same ring.
//!
//! Keys: Up/Down choose, Enter plays from the chosen track, Space or the play key
//! pauses, N or the next key skips, P or the previous key goes back, Q adds the chosen
//! track to the queue (it plays next), F11 closes.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::sound::Sound;
use sex_app::{files, Event, Window};
use sex_media::{Decoder, Next, Source};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[music.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const AUDIO: [&[u8]; 9] = [b".mp3", b".flac", b".ogg", b".oga", b".opus", b".m4a", b".aac", b".wav", b".mka"];
fn is_audio(name: &[u8]) -> bool {
    AUDIO.iter().any(|e| name.len() > e.len() && name[name.len() - e.len()..].eq_ignore_ascii_case(e))
}
const MAX_TRACKS: usize = 2000;

/// A file read through Files a piece at a time.
struct FileSource { window: *mut Window, path: Vec<u8>, len: u64, cache: Vec<u8>, cache_at: u64 }
const CACHE: usize = 128 * 1024;
impl Source for FileSource {
    fn len(&self) -> Option<u64> { Some(self.len) }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        if at >= self.len { return Some(0); }
        if !(at >= self.cache_at && at < self.cache_at + self.cache.len() as u64) {
            let want = (CACHE as u64).min(self.len - at) as usize;
            self.cache.resize(want, 0);
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

#[derive(Clone)]
struct Track { path: Vec<u8>, len: u64, title: String, artist: String, album: String, number: u32, ms: u64 }

fn open(window: &mut Window, t: &Track) -> Option<Decoder> {
    let wp: *mut Window = window;
    Decoder::open(Box::new(FileSource { window: wp, path: t.path.clone(), len: t.len, cache: Vec::new(), cache_at: 0 })).ok()
}

fn base_name(path: &[u8]) -> String {
    let name = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
    let stem = match name.iter().rposition(|b| *b == b'.') { Some(i) if i > 0 => &name[..i], _ => name };
    String::from_utf8_lossy(stem).into_owned()
}

/// Every audio file under `path` (a file, or a folder three levels down).
fn collect(window: &mut Window, path: &[u8], depth: u32, out: &mut Vec<(Vec<u8>, u64)>) {
    if out.len() >= MAX_TRACKS { return; }
    match files::stat(window, path) {
        Ok(st) if st.dir => {
            if depth > 3 { return; }
            let Ok(mut entries) = files::list(window, path) else { return };
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for e in entries {
                let mut p = path.to_vec();
                if !p.ends_with(b"/") { p.push(b'/'); }
                p.extend_from_slice(&e.name);
                if e.dir { collect(window, &p, depth + 1, out); }
                else if is_audio(&e.name) && out.len() < MAX_TRACKS { out.push((p, e.size)); }
            }
        }
        Ok(st) if is_audio(path) => out.push((path.to_vec(), st.size)),
        _ => {}
    }
}

/// Reads each file's tags; files FFmpeg cannot open are left out.
fn library(window: &mut Window, args: &[&[u8]]) -> Vec<Track> {
    let mut found = Vec::new();
    for a in args { collect(window, a, 0, &mut found); }
    let mut tracks = Vec::new();
    for (k, (path, len)) in found.into_iter().enumerate() {
        if k % 25 == 0 { window.show(format!("Music\n\nReading tags: {} files...", k).as_bytes()); }
        let t = Track { path, len, title: String::new(), artist: String::new(), album: String::new(), number: 0, ms: 0 };
        let Some(d) = open(window, &t) else { continue };
        if d.info.sample_rate == 0 { continue; }
        let number = d.tag("track").and_then(|v| v.split('/').next().and_then(|n| n.trim().parse().ok())).unwrap_or(0);
        tracks.push(Track {
            title: d.tag("title").unwrap_or_else(|| base_name(&t.path)),
            artist: d.tag("album_artist").or_else(|| d.tag("artist")).unwrap_or_else(|| String::from("Unknown artist")),
            album: d.tag("album").unwrap_or_else(|| String::from("Unknown album")),
            number, ms: d.info.duration_ms, ..t
        });
    }
    tracks.sort_by(|a, b| (a.artist.to_lowercase(), a.album.to_lowercase(), a.number, &a.path)
        .cmp(&(b.artist.to_lowercase(), b.album.to_lowercase(), b.number, &b.path)));
    tracks
}

fn clock(ms: u64) -> String { format!("{}:{:02}", ms / 60_000, ms / 1000 % 60) }

/// Plays tracks back to back through one sound ring.
struct Engine {
    sound: Sound,
    /// The track playing (being decoded) and its decoder.
    cur: Option<(usize, Decoder)>,
    /// The one after it, opened ahead so its first sound follows at once.
    next: Option<(usize, Decoder)>,
    pcm: Vec<[i16; 2]>,
    left: (usize, usize),
    pic: Vec<u32>,
    paused: bool,
    /// Ring position where each track's sound began: (frame, track).
    starts: Vec<(u64, usize)>,
}

impl Engine {
    /// The track the listener hears now: the last one whose first frame has played.
    fn hearing(&self) -> Option<usize> {
        let played = self.sound.played_total();
        self.starts.iter().rev().find(|(f, _)| *f <= played).or(self.starts.first()).map(|s| s.1)
    }
    fn stop(&mut self) {
        self.cur = None;
        self.next = None;
        self.left = (0, 0);
        self.starts.clear();
        self.sound.skip();
    }
}

struct App {
    tracks: Vec<Track>,
    chosen: usize,
    queue: Vec<usize>,
    shown: String,
}

impl App {
    /// The track after `i`: the queue first, then the library's order.
    fn after(&mut self, i: usize) -> Option<usize> {
        if !self.queue.is_empty() { return Some(self.queue.remove(0)); }
        (i + 1 < self.tracks.len()).then_some(i + 1)
    }

    fn render(&mut self, window: &mut Window, e: &Engine) {
        let mut s = String::new();
        let hearing = e.hearing();
        let albums = {
            let mut a: Vec<(&str, &str)> = self.tracks.iter().map(|t| (t.artist.as_str(), t.album.as_str())).collect();
            a.dedup();
            a.len()
        };
        s.push_str(&format!("Music  {} tracks, {} albums", self.tracks.len(), albums));
        if let Some(h) = hearing {
            let t = &self.tracks[h];
            s.push_str(&format!("\n{} {} - {}", if e.paused { "Paused:" } else { "Playing:" }, t.artist, t.title));
        }
        if !self.queue.is_empty() { s.push_str(&format!("   ({} queued)", self.queue.len())); }
        s.push('\n');
        // A window of lines around the chosen track.
        let rows = 18usize;
        let top = self.chosen.saturating_sub(rows / 2).min(self.tracks.len().saturating_sub(rows));
        let mut last_album: Option<(&str, &str)> = None;
        for (i, t) in self.tracks.iter().enumerate().skip(top).take(rows) {
            let album = (t.artist.as_str(), t.album.as_str());
            if last_album != Some(album) { s.push_str(&format!("\n{} - {}\n", t.artist, t.album)); last_album = Some(album); }
            let mark = if Some(i) == hearing { if e.paused { "||" } else { "> " } } else { "  " };
            let pick = if i == self.chosen { "*" } else { " " };
            let num = if t.number > 0 { format!("{:2}.", t.number) } else { String::from("   ") };
            s.push_str(&format!("{}{}{} {}  {}\n", pick, mark, num, t.title, clock(t.ms)));
        }
        s.push_str("\nEnter play  Space pause  N/P next/previous  Q queue  F11 close");
        if s != self.shown {
            window.show(s.as_bytes());
            self.shown = s;
        }
    }
}

fn start_track(e: &mut Engine, app: &mut App, i: usize, wp: *mut Window) {
    e.stop();
    e.paused = false;
    e.sound.pause(false);
    let w = unsafe { &mut *wp };
    e.cur = open(w, &app.tracks[i]).map(|d| (i, d));
    e.starts.push((e.sound.written_total(), i));
    sex_pdx::serial_record!("[music.play] n={} title={}", i, app.tracks[i].title);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Music");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let words: Vec<Vec<u8>> = given.split(|b| *b == b' ').filter(|w| !w.is_empty()).map(|w| w.to_vec()).collect();
    let refs: Vec<&[u8]> = words.iter().map(|w| w.as_slice()).collect();
    if refs.is_empty() {
        window.show(b"Music\n\nUsage: run music @FOLDER (or @SONG.flac @SONG.mp3 ...)");
        loop { if let Event::Close = window.next() { window.close() } }
    }
    let tracks = library(&mut window, &refs);
    sex_pdx::serial_record!("[music.library] tracks={}", tracks.len());
    for (i, t) in tracks.iter().enumerate().take(64) {
        sex_pdx::serial_record!("[music.library.track] n={} artist={} album={} number={} title={} ms={}", i, t.artist, t.album, t.number, t.title, t.ms);
    }
    let Some(sound) = Sound::open() else {
        window.show(b"Music\n\nNo sound for this program.");
        loop { if let Event::Close = window.next() { window.close() } }
    };
    let mut app = App { tracks, chosen: 0, queue: Vec::new(), shown: String::new() };
    let mut e = Engine { sound, cur: None, next: None, pcm: alloc::vec![[0i16; 2]; 4096], left: (0, 0), pic: alloc::vec![0u32; 16 * 16], paused: false, starts: Vec::new() };
    let wp: *mut Window = &mut window;
    let window = unsafe { &mut *wp };
    // Plays from the first track at once.
    let start = move |e: &mut Engine, app: &mut App, i: usize| start_track(e, app, i, wp);
    if !app.tracks.is_empty() { start(&mut e, &mut app, 0); }
    let mut last_heard = None;
    loop {
        // Keys.
        while let Some(ev) = window.poll() {
            match ev {
                Event::Close => { e.stop(); e.sound.close(); sex_pdx::task_finish(0) }
                Event::Key(k) => match k.code {
                    0x48 => app.chosen = app.chosen.saturating_sub(1),
                    0x50 => if app.chosen + 1 < app.tracks.len() { app.chosen += 1 },
                    0x1C if !app.tracks.is_empty() => { let i = app.chosen; start(&mut e, &mut app, i) }
                    57 | sex_pdx::app_key::KEY_PLAY_PAUSE => {
                        e.paused = !e.paused;
                        e.sound.pause(e.paused);
                        sex_pdx::serial_record!("[music.paused] {}", e.paused as u8);
                    }
                    0x31 | sex_pdx::app_key::KEY_NEXT => {
                        let now = e.hearing().unwrap_or(0);
                        if let Some(n) = app.after(now) { start(&mut e, &mut app, n) }
                    }
                    0x19 | sex_pdx::app_key::KEY_PREVIOUS => {
                        let now = e.hearing().unwrap_or(0);
                        start(&mut e, &mut app, now.saturating_sub(1))
                    }
                    0x10 if !app.tracks.is_empty() => {
                        app.queue.push(app.chosen);
                        sex_pdx::serial_record!("[music.queued] n={} queue={}", app.chosen, app.queue.len());
                    }
                    sex_pdx::app_key::KEY_STOP => { e.stop(); e.paused = true; }
                    _ => {}
                },
                _ => {}
            }
        }
        // Keep the ring fed, track after track.
        let mut wrote = false;
        while !e.paused && e.cur.is_some() {
            if e.left.0 == e.left.1 {
                let (i, d) = e.cur.as_mut().unwrap();
                let i = *i;
                // Open the next track early, so it is ready when this one ends.
                if e.next.is_none() {
                    if let Some(n) = app.after(i) {
                        e.next = open(unsafe { &mut *wp }, &app.tracks[n]).map(|d| (n, d));
                        if e.next.is_none() { sex_pdx::serial_record!("[music.skip] n={} reason=open", n); }
                    }
                }
                match d.next(&mut e.pic, 16, 16, &mut e.pcm) {
                    Ok(Next::Sound { frames, .. }) => e.left = (0, frames),
                    Ok(Next::Picture { .. }) => continue,
                    Ok(Next::Later) => break,
                    Ok(Next::End) | Err(_) => {
                        // The next track's first frame goes right after this one's last.
                        e.cur = e.next.take();
                        if let Some((n, _)) = e.cur {
                            e.starts.push((e.sound.written_total(), n));
                            sex_pdx::serial_record!("[music.next] n={} at_frame={} title={}", n, e.sound.written_total(), app.tracks[n].title);
                        } else {
                            sex_pdx::serial_record!("[music.end] at_frame={}", e.sound.written_total());
                        }
                        continue;
                    }
                }
            }
            let took = e.sound.write(&e.pcm[e.left.0..e.left.1]);
            e.left.0 += took;
            wrote |= took > 0;
            if took == 0 { break; }
        }
        let heard = e.hearing();
        if heard != last_heard {
            if let Some(h) = heard { sex_pdx::serial_record!("[music.hearing] n={} played={}", h, e.sound.played_total()); }
            last_heard = heard;
        }
        app.render(window, &e);
        if !wrote { let _ = sex_pdx::sys_wait_self_ticks(15); }
    }
}
