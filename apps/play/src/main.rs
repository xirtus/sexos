// SPDX-License-Identifier: GPL-3.0-only
//! Play: plays video and sound. `run play @clip.mp4`. FFmpeg (libavformat, libavcodec,
//! libswscale, libswresample) runs here, inside the program's own confinement, linked
//! against sex-libc: the C code holds exactly what this program's BOOT row holds and
//! reads the file only through the callbacks below. A hostile file can only crash
//! this window.
//!
//! Formats: MP4/MOV, Matroska/WebM, Ogg, MP3, AAC, FLAC and WAV, with H.264, HEVC,
//! VP8, VP9 and MPEG-4 video and AAC, MP3, Opus, Vorbis, FLAC and PCM sound. Sound
//! goes to Bell through this program's sound ring and sets the pace: each picture
//! waits in a short queue for the sound clock to reach its time. Without sound, ticks set the pace.
#![no_std]
#![no_main]

extern crate alloc;
extern crate sex_libc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use sex_app::sound::{Sound, RATE};
use sex_app::{files, Event, Window};
use sex_pdx::pixel_region::{MAX_H, MAX_W};

#[repr(C)]
struct Info {
    width: c_int,
    height: c_int,
    fps_num: c_int,
    fps_den: c_int,
    duration_ms: i64,
    codec: [c_char; 16],
    sample_rate: c_int,
    channels: c_int,
    audio_codec: [c_char; 16],
    format: [c_char; 16],
}

type ReadFn = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type SeekFn = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;

extern "C" {
    fn sx_open(opaque: *mut c_void, read: ReadFn, seek: SeekFn, info: *mut Info, err: *mut c_int) -> *mut c_void;
    fn sx_next(s: *mut c_void, out: *mut u32, w: c_int, h: c_int, pcm: *mut i16, cap: c_int,
               frames: *mut c_int, at_ms: *mut i64) -> c_int;
    fn sx_close(s: *mut c_void);
    fn sx_error(err: c_int, buf: *mut c_char, len: c_int);
}

/// The file, read through Files a piece at a time, so its size is not limited by
/// memory: FFmpeg sees one byte stream with `read` and `seek`.
struct Source {
    window: *mut Window,
    path: Vec<u8>,
    len: u64,
    at: u64,
    cache: Vec<u8>,
    cache_at: u64,
}

const CACHE: usize = 256 * 1024;
const AVERROR_EOF: c_int = -0x2046_4f45; // FFERRTAG('E','O','F',' ')
const AVERROR_EIO: c_int = -5;
const AVSEEK_SIZE: c_int = 0x10000;

unsafe extern "C" fn read(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let src = unsafe { &mut *opaque.cast::<Source>() };
    if src.at >= src.len {
        return AVERROR_EOF;
    }
    let in_cache = src.at >= src.cache_at && src.at < src.cache_at + src.cache.len() as u64;
    if !in_cache {
        let want = (CACHE as u64).min(src.len - src.at) as usize;
        src.cache.resize(want, 0);
        let window = unsafe { &mut *src.window };
        match files::read_at(window, &src.path, src.at, &mut src.cache) {
            Ok(got) if got > 0 => { src.cache.truncate(got); src.cache_at = src.at; }
            _ => return AVERROR_EIO,
        }
    }
    let off = (src.at - src.cache_at) as usize;
    let n = (size.max(0) as usize).min(src.cache.len() - off);
    unsafe { core::ptr::copy_nonoverlapping(src.cache.as_ptr().add(off), buf, n) };
    src.at += n as u64;
    n as c_int
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let src = unsafe { &mut *opaque.cast::<Source>() };
    let len = src.len as i64;
    if whence & AVSEEK_SIZE != 0 {
        return len;
    }
    let base = match whence & 3 { 0 => 0, 1 => src.at as i64, 2 => len, _ => return -1 };
    let to = base + offset;
    if to < 0 || to > len {
        return -1;
    }
    src.at = to as u64;
    to
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[play.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn log(bytes: &[u8]) {
    sex_pdx::serial_record!("[play.ffmpeg] {}", core::str::from_utf8(bytes).unwrap_or("?").trim_end());
}

fn exit(code: c_int) -> ! {
    sex_pdx::task_finish(code as u64)
}

fn clock() -> (i64, u64) {
    let ms = sex_pdx::get_ticks();
    (0, ms * 1_000_000)
}

fn say(window: &mut Window, why: &[u8]) {
    let mut text = [0u8; 300];
    let head = b"Play. F11 closes.\n\n";
    let n = (head.len() + why.len()).min(text.len());
    text[..head.len()].copy_from_slice(head);
    text[head.len()..n].copy_from_slice(&why[..n - head.len()]);
    window.show(&text[..n]);
}

fn fail(window: &mut Window, why: &[u8]) {
    sex_pdx::serial_record!("[play.failed] {}", core::str::from_utf8(why).unwrap_or("?"));
    say(window, why);
}

/// The largest size with the video's aspect ratio that fits the window.
fn fit(w: usize, h: usize) -> (usize, usize) {
    if w <= MAX_W && h <= MAX_H {
        return (w, h);
    }
    if w * MAX_H >= h * MAX_W { (MAX_W, (h * MAX_W / w).max(1)) } else { ((w * MAX_H / h).max(1), MAX_H) }
}

fn name(raw: &[c_char; 16]) -> &str {
    let n = raw.iter().position(|&c| c == 0).unwrap_or(16);
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), n) }).unwrap_or("?")
}

/// Ends the run if the person closed the window.
fn closing(window: &mut Window, s: *mut c_void, pace: &mut Pace, shown: u32) {
    if let Some(Event::Close) = window.poll() {
        // Bell stops the stream once the queued sound is played.
        if let Some(sound) = pace.sound.as_mut() { sound.close(); }
        unsafe { sx_close(s) };
        sex_pdx::serial_record!("[play.closed] target={} task={} frames={}", window.target, window.task, shown);
        sex_pdx::task_finish(0);
    }
}

/// What sets the pace: the sound card's clock while sound plays, ticks otherwise.
/// The card reports in whole periods (about 43 ms), so between reports the clock
/// runs on ticks from the last one, never more than a period ahead of it.
struct Pace {
    start: u64,
    sound: Option<Sound>,
    started: bool,
    /// Ticks when PLAYED last moved, and what it read then.
    moved: u64,
    last_played: u64,
    /// The media time the sound clock began at.
    base_ms: i64,
    /// The last time this clock gave, and the tick it gave it at.
    last_ms: i64,
    last_tick: u64,
}

impl Pace {
    fn new() -> Pace {
        let t = sex_pdx::get_ticks();
        Pace { start: t, sound: None, started: false, moved: t, last_played: 0, base_ms: 0, last_ms: 0, last_tick: t }
    }
    /// Media time now, in ms. It never goes backwards.
    fn now(&mut self) -> i64 {
        let ticks = sex_pdx::get_ticks();
        let by_ticks = self.last_ms + (ticks - self.last_tick) as i64;
        let mut at = by_ticks;
        if let (Some(sound), true) = (&self.sound, self.started) {
            let played = sound.played();
            if played != self.last_played {
                self.last_played = played;
                self.moved = ticks;
            }
            let pending = sound.pending();
            if pending > 0 && ticks.saturating_sub(self.moved) > 750 {
                // Sound is queued and nothing plays: no card, or a stuck one. Go on
                // in silence on ticks.
                sex_pdx::serial_record!("[play.sound.stalled] played={} pending={} at_ms={}", played, pending, by_ticks);
                self.sound = None;
            } else if pending > 0 {
                let heard = self.base_ms + (played * 1000 / RATE as u64) as i64;
                at = heard + ((ticks - self.moved) as i64).min(43);
            }
            // With nothing queued (before the first sound, or a gap), ticks carry on.
        }
        let at = at.max(self.last_ms);
        self.last_ms = at;
        self.last_tick = ticks;
        at
    }
}

struct Played { pictures: u32, sound_frames: u64 }

/// A decoded picture waiting for its time.
struct Pic { due: i64, px: Vec<u32> }

struct Shown { n: u32, late_ms: i64 }

const QUEUE_BYTES: usize = 32 * 1024 * 1024;
const QUEUE_MAX: usize = 16;
/// Silence that goes before sound starting later than the pictures; at most this long.
const PAD_MAX_MS: i64 = 30_000;
static SILENCE: [[i16; 2]; 1024] = [[0; 2]; 1024];

/// Shows every queued picture whose time has come, oldest first.
fn present(window: &mut Window, queue: &mut VecDeque<Pic>, free: &mut Vec<Vec<u32>>, pace: &mut Pace,
           shown: &mut Shown, w: usize, h: usize) {
    while let Some(front) = queue.front() {
        let now = pace.now();
        if now < front.due {
            return;
        }
        shown.late_ms = shown.late_ms.max(now - front.due);
        let pic = queue.pop_front().unwrap();
        window.pixels()[..w * h].copy_from_slice(&pic.px);
        window.show_pixels(w, h);
        let n = shown.n;
        if n < 3 || n % 10 == 0 {
            let hash = pic.px.iter().fold(0xcbf2_9ce4_8422_2325u64, |a, &p| (a ^ p as u64).wrapping_mul(0x100_0000_01b3));
            sex_pdx::serial_record!("[play.frame] n={} at_ms={} hash={:016x} probe={:06x}", n, pic.due, hash, pic.px[(h / 2) * w + 3 * w / 4] & 0xFF_FFFF);
        }
        shown.n += 1;
        free.push(pic.px);
    }
}

/// Plays the stream; returns what was shown and heard, or FFmpeg's error.
fn play(window: &mut Window, src: &mut Source) -> Result<Played, c_int> {
    let mut info: Info = unsafe { core::mem::zeroed() };
    let mut err = 0;
    let s = unsafe { sx_open((src as *mut Source).cast(), read, seek, &mut info, &mut err) };
    if s.is_null() {
        return Err(err);
    }
    let pictures = info.width > 0 && info.height > 0;
    let (w, h) = if pictures { fit(info.width as usize, info.height as usize) } else { (0, 0) };
    let (num, den) = if info.fps_num > 0 && info.fps_den > 0 { (info.fps_num as u64, info.fps_den as u64) } else { (25, 1) };
    sex_pdx::serial_record!(
        "[play.opened] format={} codec={} w={} h={} fps={}/{} ms={} shown={}x{} audio={} rate={} channels={}",
        name(&info.format), name(&info.codec), info.width, info.height, num, den, info.duration_ms, w, h,
        name(&info.audio_codec), info.sample_rate, info.channels
    );
    let mut pace = Pace::new();
    if info.sample_rate > 0 {
        pace.sound = Sound::open();
        if pace.sound.is_none() {
            sex_pdx::serial_record!("[play.sound.none] reason=no_ring");
        }
    }
    if !pictures {
        let mut text = Vec::new();
        text.extend_from_slice(b"Playing sound: ");
        text.extend_from_slice(name(&info.audio_codec).as_bytes());
        text.extend_from_slice(b" in ");
        text.extend_from_slice(name(&info.format).as_bytes());
        say(window, &text);
    }
    // Pictures are decoded ahead into a queue, so sound keeps flowing to the ring
    // while each picture waits for its time. The queue holds about half a second,
    // within a memory budget.
    let bytes = (w * h * 4).max(1);
    let cap = (QUEUE_BYTES / bytes).clamp(2, QUEUE_MAX);
    let mut queue: VecDeque<Pic> = VecDeque::new();
    let mut free: Vec<Vec<u32>> = Vec::new();
    const CAP: usize = 16384;
    let mut pcm: Vec<[i16; 2]> = alloc::vec![[0; 2]; CAP];
    let mut shown = Shown { n: 0, late_ms: 0 };
    let mut decoded = 0u64;
    let mut heard = 0u64;
    let mut eof = false;
    loop {
        closing(window, s, &mut pace, shown.n);
        present(window, &mut queue, &mut free, &mut pace, &mut shown, w, h);
        if eof && queue.is_empty() {
            break;
        }
        if eof || queue.len() >= cap {
            let now = pace.now();
            if let Some(front) = queue.front() {
                if now < front.due {
                    let _ = sex_pdx::sys_wait_self_ticks(((front.due - now) as u64).clamp(1, 16));
                }
            }
            continue;
        }
        let mut px = match free.pop() {
            Some(px) => px,
            None => {
                let mut px = Vec::new();
                if px.try_reserve_exact(w * h).is_err() {
                    unsafe { sx_close(s) };
                    return Err(-12); // AVERROR(ENOMEM)
                }
                px.resize(w * h, 0u32);
                px
            }
        };
        let (mut frames, mut at): (c_int, i64) = (0, -1);
        let r = unsafe { sx_next(s, px.as_mut_ptr(), w as c_int, h as c_int, pcm.as_mut_ptr().cast(), CAP as c_int, &mut frames, &mut at) };
        if r < 0 {
            unsafe { sx_close(s) };
            return Err(r);
        }
        if r == 1 {
            let due = if at >= 0 { at } else { (decoded * 1000 * den / num) as i64 };
            decoded += 1;
            queue.push_back(Pic { due, px });
            continue;
        }
        free.push(px);
        if r == 0 {
            eof = true;
            continue;
        }
        if pace.sound.is_none() {
            continue;
        }
        let mut pad = 0u64;
        if !pace.started {
            // The sound clock reads this stream time once its first frame plays. Sound
            // that starts later than now waits behind silence, so it keeps its place.
            let now = pace.now();
            let at = at.max(0);
            pace.started = true;
            pace.base_ms = at.min(now);
            pad = ((at - at.min(now)).min(PAD_MAX_MS) as u64) * RATE as u64 / 1000;
            pace.moved = sex_pdx::get_ticks();
        }
        let mut left = &pcm[..frames as usize];
        while pad > 0 || !left.is_empty() {
            let Some(sound) = pace.sound.as_mut() else { break };
            let blocked = if pad > 0 {
                let n = sound.write(&SILENCE[..pad.min(SILENCE.len() as u64) as usize]);
                pad -= n as u64;
                n == 0
            } else {
                let n = sound.write(left);
                left = &left[n..];
                heard += n as u64;
                !left.is_empty()
            };
            if blocked {
                // The ring is full: show what is due while it drains.
                closing(window, s, &mut pace, shown.n);
                present(window, &mut queue, &mut free, &mut pace, &mut shown, w, h);
                let _ = sex_pdx::sys_wait_self_ticks(5);
            }
            // The card may have stopped; `now` notices and drops sound.
            let _ = pace.now();
        }
    }
    let late_ms = shown.late_ms;
    let shown = shown.n;
    // Let the last sound play out before saying it is done.
    if let Some(sound) = pace.sound.as_mut() {
        sound.close();
        let end = sex_pdx::get_ticks() + 3000;
        while pace.sound.as_ref().is_some_and(|s| s.pending() > 0) && sex_pdx::get_ticks() < end {
            closing(window, s, &mut pace, shown);
            let _ = pace.now();
            let _ = sex_pdx::sys_wait_self_ticks(10);
        }
    }
    unsafe { sx_close(s) };
    let ms = sex_pdx::get_ticks() - pace.start;
    let played = pace.sound.as_ref().map_or(0, |s| s.played());
    sex_pdx::serial_record!("[play.done] frames={} ms={} w={} h={} sound_frames={} played={} late_ms={}", shown, ms, w, h, heard, played, late_ms);
    Ok(Played { pictures: shown, sound_frames: heard })
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_libc::set_log(log);
    sex_libc::set_exit(exit);
    sex_libc::time::set_clock(clock);
    let mut window = Window::open();
    window.title(b"Play");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let file = given.split(|b| *b == b' ').find(|w| !w.is_empty()).map(|f| f.to_vec());
    match file {
        None => fail(&mut window, b"Usage: run play @VIDEO.mp4"),
        Some(path) => match files::size(&mut window, &path) {
            Err(e) => fail(&mut window, files::message(e)),
            Ok(len) => {
                say(&mut window, b"Opening...");
                let wp: *mut Window = &mut window;
                let mut src = Source { window: wp, path, len, at: 0, cache: Vec::new(), cache_at: 0 };
                match play(unsafe { &mut *wp }, &mut src) {
                    Ok(p) if p.pictures == 0 && p.sound_frames == 0 => fail(&mut window, b"That file has nothing to play."),
                    Ok(p) if p.pictures == 0 => say(&mut window, b"Finished playing."),
                    Ok(_) => {}
                    Err(e) => {
                        let mut text = [0 as c_char; 96];
                        unsafe { sx_error(e, text.as_mut_ptr(), text.len() as c_int) };
                        let n = text.iter().position(|&c| c == 0).unwrap_or(0);
                        let mut why = Vec::new();
                        why.extend_from_slice(b"Cannot play that file: ");
                        why.extend(text[..n].iter().map(|&c| c as u8));
                        fail(&mut window, &why);
                    }
                }
            }
        },
    }
    sex_pdx::serial_record!("[play.ready] target={} task={}", window.target, window.task);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[play.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
