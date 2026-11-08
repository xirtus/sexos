// SPDX-License-Identifier: GPL-3.0-only
//! Play: plays a video. `run play @clip.mp4`. FFmpeg (libavformat, libavcodec,
//! libswscale) runs here, inside the program's own confinement, linked against
//! sex-libc: the C code holds exactly what this program's BOOT row holds and reads
//! the file only through the callbacks below. A hostile file can only crash this
//! window. Video only for now: MP4/MOV with H.264.
#![no_std]
#![no_main]

extern crate alloc;
extern crate sex_libc;

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use sex_app::{files, Event, Window};
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

/// Largest file Play reads; the rest of the heap is the decoder's.
const MAX_FILE: usize = 3 * 1024 * 1024 / 2;

#[repr(C)]
struct Info {
    width: c_int,
    height: c_int,
    fps_num: c_int,
    fps_den: c_int,
    duration_ms: i64,
    codec: [c_char; 16],
}

type ReadFn = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type SeekFn = unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64;

extern "C" {
    fn sx_open(opaque: *mut c_void, read: ReadFn, seek: SeekFn, info: *mut Info, err: *mut c_int) -> *mut c_void;
    fn sx_next(s: *mut c_void, out: *mut u32, w: c_int, h: c_int) -> c_int;
    fn sx_close(s: *mut c_void);
    fn sx_error(err: c_int, buf: *mut c_char, len: c_int);
}

/// The file in memory, read by FFmpeg through `read` and `seek`.
struct Source {
    bytes: Vec<u8>,
    at: usize,
}

const AVERROR_EOF: c_int = -0x2046_4f45; // FFERRTAG('E','O','F',' ')
const AVSEEK_SIZE: c_int = 0x10000;

unsafe extern "C" fn read(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let src = unsafe { &mut *opaque.cast::<Source>() };
    let n = (size.max(0) as usize).min(src.bytes.len() - src.at);
    if n == 0 {
        return AVERROR_EOF;
    }
    unsafe { core::ptr::copy_nonoverlapping(src.bytes.as_ptr().add(src.at), buf, n) };
    src.at += n;
    n as c_int
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let src = unsafe { &mut *opaque.cast::<Source>() };
    let len = src.bytes.len() as i64;
    if whence & AVSEEK_SIZE != 0 {
        return len;
    }
    let base = match whence & 3 { 0 => 0, 1 => src.at as i64, 2 => len, _ => return -1 };
    let to = base + offset;
    if to < 0 || to > len {
        return -1;
    }
    src.at = to as usize;
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
    let mut text = [0u8; 200];
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

/// Plays the stream; returns frames shown, or FFmpeg's error.
fn play(window: &mut Window, bytes: Vec<u8>) -> Result<u32, c_int> {
    let mut src = Source { bytes, at: 0 };
    let mut info = Info { width: 0, height: 0, fps_num: 0, fps_den: 0, duration_ms: 0, codec: [0; 16] };
    let mut err = 0;
    let s = unsafe { sx_open((&mut src as *mut Source).cast(), read, seek, &mut info, &mut err) };
    if s.is_null() {
        return Err(err);
    }
    let codec_len = info.codec.iter().position(|&c| c == 0).unwrap_or(16);
    let codec: Vec<u8> = info.codec[..codec_len].iter().map(|&c| c as u8).collect();
    let (w, h) = fit(info.width.max(1) as usize, info.height.max(1) as usize);
    let (num, den) = if info.fps_num > 0 && info.fps_den > 0 { (info.fps_num as u64, info.fps_den as u64) } else { (25, 1) };
    sex_pdx::serial_record!(
        "[play.opened] codec={} w={} h={} fps={}/{} ms={} shown={}x{}",
        core::str::from_utf8(&codec).unwrap_or("?"), info.width, info.height, num, den, info.duration_ms, w, h
    );
    let out = &mut window.pixels()[..PIXELS];
    let start = sex_pdx::get_ticks();
    let mut frames = 0u32;
    loop {
        let r = unsafe { sx_next(s, out.as_mut_ptr(), w as c_int, h as c_int) };
        if r < 0 {
            unsafe { sx_close(s) };
            return Err(r);
        }
        if r == 0 {
            break;
        }
        // Each frame at its time: frame n at n * den / num seconds.
        let due = start + frames as u64 * 1000 * den / num;
        loop {
            if let Some(Event::Close) = window.poll() {
                unsafe { sx_close(s) };
                sex_pdx::serial_record!("[play.closed] target={} task={} frames={}", window.target, window.task, frames);
                sex_pdx::task_finish(0);
            }
            let now = sex_pdx::get_ticks();
            if now >= due {
                break;
            }
            let _ = sex_pdx::sys_wait_self_ticks((due - now).clamp(1, 64));
        }
        window.show_pixels(w, h);
        if frames < 3 || frames % 10 == 0 {
            let hash = out[..w * h].iter().fold(0xcbf2_9ce4_8422_2325u64, |a, &p| (a ^ p as u64).wrapping_mul(0x100_0000_01b3));
            sex_pdx::serial_record!("[play.frame] n={} hash={:016x} probe={:06x}", frames, hash, out[(h / 2) * w + 3 * w / 4] & 0xFF_FFFF);
        }
        frames += 1;
    }
    unsafe { sx_close(s) };
    let ms = sex_pdx::get_ticks() - start;
    sex_pdx::serial_record!("[play.done] frames={} ms={} w={} h={}", frames, ms, w, h);
    Ok(frames)
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
    let file = given.split(|b| *b == b' ').find(|w| !w.is_empty());
    match file {
        None => fail(&mut window, b"Usage: run play @VIDEO.mp4"),
        Some(file) => {
            let mut bytes = Vec::new();
            if bytes.try_reserve_exact(MAX_FILE).is_err() {
                fail(&mut window, b"Not enough memory to read the video.");
            } else {
                bytes.resize(MAX_FILE, 0);
                match files::read(&mut window, file, &mut bytes) {
                    Err(e) => fail(&mut window, files::message(e)),
                    Ok(len) if len == MAX_FILE => fail(&mut window, b"That video is larger than 1.5 MiB."),
                    Ok(len) => {
                        bytes.truncate(len);
                        bytes.shrink_to_fit();
                        say(&mut window, b"Opening...");
                        match play(&mut window, bytes) {
                            Ok(0) => fail(&mut window, b"That video has no pictures."),
                            Ok(_) => {}
                            Err(e) => {
                                let mut text = [0 as c_char; 96];
                                unsafe { sx_error(e, text.as_mut_ptr(), text.len() as c_int) };
                                let n = text.iter().position(|&c| c == 0).unwrap_or(0);
                                let mut why = [0u8; 128];
                                let head = b"Cannot play that video: ";
                                why[..head.len()].copy_from_slice(head);
                                for (i, &c) in text[..n].iter().enumerate() {
                                    why[head.len() + i] = c as u8;
                                }
                                fail(&mut window, &why[..head.len() + n]);
                            }
                        }
                    }
                }
            }
        }
    }
    sex_pdx::serial_record!("[play.ready] target={} task={}", window.target, window.task);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[play.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
