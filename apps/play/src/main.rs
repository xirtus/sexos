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
    match file {
        None if words.first().is_some_and(|w| w.starts_with(b"--")) => {}
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
