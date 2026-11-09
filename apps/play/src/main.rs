// SPDX-License-Identifier: GPL-3.0-only
//! Play: plays video and sound. `run play @clip.mp4`. Decoding is sex-media's
//! (FFmpeg inside this program's own confinement): this program reads the file
//! through Files, shows each picture in its window and handles keys. A hostile file
//! can only crash this window.
//!
//! Keys: Space or K pauses and plays; Left/J and Right/L go back and forward 5 s;
//! Home goes to the start. F11 closes.
#![no_std]
#![no_main]

extern crate alloc;

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
    let head = b"Play. Space pauses, J/L or Left/Right seek, F11 closes.\n\n";
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

/// Handles keys and close; false when the person closed the window.
fn keys(window: &mut Window, player: &mut Player) -> bool {
    while let Some(e) = window.poll() {
        match e {
            Event::Close => return false,
            Event::Key(k) => match k.code {
                57 | 37 => {
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
                _ => {}
            },
            _ => {}
        }
    }
    true
}

struct Played { pictures: u32, sound_frames: u64 }

/// Plays the stream; returns what was shown and heard, or FFmpeg's error.
fn play(window: &mut Window, src: FileSource) -> Result<Played, sex_media::Error> {
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
    loop {
        if !keys(window, &mut player) {
            sex_pdx::serial_record!("[play.closed] target={} task={} frames={}", window.target, window.task, player.shown);
            drop(player);
            sex_pdx::task_finish(0);
        }
        if let (Some(sex_media::Silence::Stalled { played, pending }), false) = (player.silence(), stalled) {
            stalled = true;
            sex_pdx::serial_record!("[play.sound.stalled] played={} pending={}", played, pending);
        }
        match player.step() {
            Step::Picture { n, at_ms } => {
                let px = player.picture();
                window.pixels()[..w * h].copy_from_slice(px);
                window.show_pixels(w, h);
                if n < 3 || n % 10 == 0 {
                    let hash = px.iter().fold(0xcbf2_9ce4_8422_2325u64, |a, &p| (a ^ p as u64).wrapping_mul(0x100_0000_01b3));
                    sex_pdx::serial_record!("[play.frame] n={} at_ms={} hash={:016x} probe={:06x}", n, at_ms, hash, px[(h / 2) * w + 3 * w / 4] & 0xFF_FFFF);
                }
            }
            Step::Wait { ms } => { let _ = sex_pdx::sys_wait_self_ticks(ms.clamp(1, 16)); }
            Step::Paused => { let _ = sex_pdx::sys_wait_self_ticks(30); }
            Step::Ended => break,
        }
    }
    if let Some(e) = player.error() {
        return Err(e);
    }
    let ms = sex_pdx::get_ticks() - begun;
    sex_pdx::serial_record!("[play.done] frames={} ms={} w={} h={} sound_frames={} played={} late_ms={}",
        player.shown, ms, w, h, player.heard, player.played(), player.late_ms);
    Ok(Played { pictures: player.shown, sound_frames: player.heard })
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
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
                let src = FileSource { window: wp, path, len, cache: Vec::new(), cache_at: 0 };
                match play(unsafe { &mut *wp }, src) {
                    Ok(p) if p.pictures == 0 && p.sound_frames == 0 => fail(&mut window, b"That file has nothing to play."),
                    Ok(p) if p.pictures == 0 => say(&mut window, b"Finished playing."),
                    Ok(_) => {}
                    Err(e) => {
                        let mut text = [0u8; 96];
                        let mut why = Vec::new();
                        why.extend_from_slice(b"Cannot play that file: ");
                        why.extend_from_slice(e.text(&mut text).as_bytes());
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
