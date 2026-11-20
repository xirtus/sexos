// SPDX-License-Identifier: GPL-3.0-only
//! Media bench: how fast FFmpeg (and dav1d, x264, libvpx, libaom) decode and encode
//! on this machine, with SSE only or with AVX/AVX2/FMA3.
//!
//!   run mediabench [--sse|--avx2] [--encode] [--frames N] [--size WxH] @FILE...
//!
//! Each file is read into memory and every picture of its first video stream (or
//! every sound frame) decoded, then scaled into 1280x720 BGRA (or resampled to 48 kHz
//! stereo), as Qupid shows it, timed apart; with --encode, synthetic pictures are encoded with each
//! encoder this build has (flac, libx264, libvpx-vp9, libaom-av1). --sse hides AVX
//! and up from every codec for the whole run, --avx2 hides only AVX-512 (libvpx and
//! libaom choose once per program), so a gate runs it twice and compares. Each result is a serial record:
//!
//!   [mediabench.cpu] simd=sse|avx2|full flags=0x.. avx=0|1 avx2=0|1 fma3=0|1 xcr0=0x..
//!   [mediabench.decode] file=.. codec=.. simd=.. frames=.. w=.. h=.. bytes=.. cycles=.. convert_cycles=.. ms=.. hash=..
//!   [mediabench.encode] codec=.. simd=.. frames=.. w=.. h=.. bytes=.. cycles=.. ms=.. hash=..
//!   [mediabench.done] simd=.. decoded=.. encoded=.. failed=..
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt::Write as _;
use sex_app::{files, Event, Window};
use sex_media::bench;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[mediabench.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// XCR0, or 0 when the kernel has not turned on XSAVE.
fn xcr0() -> u64 {
    let l1 = core::arch::x86_64::__cpuid(1).ecx;
    if l1 & (1 << 27) == 0 { return 0; }
    let (lo, hi): (u32, u32);
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") lo, out("edx") hi, options(nomem, nostack)); }
    (hi as u64) << 32 | lo as u64
}

fn message(e: files::Error) -> &'static str {
    core::str::from_utf8(files::message(e)).unwrap_or("read failed")
}

fn read_all(window: &mut Window, path: &[u8]) -> Result<Vec<u8>, &'static str> {
    let len = files::size(window, path).map_err(message)? as usize;
    let mut data = alloc::vec![0u8; len];
    let mut at = 0;
    while at < len {
        let end = (at + (1 << 20)).min(len);
        match files::read_at(window, path, at as u64, &mut data[at..end]) {
            Ok(n) if n > 0 => at += n,
            Ok(_) => return Err("short read"),
            Err(e) => return Err(message(e)),
        }
    }
    Ok(data)
}

/// FFERRTAG(0xF8, 'E', 'N', 'C').
const AVERROR_ENCODER_NOT_FOUND: i32 = -0x434E_45F8;

fn number(w: Option<&&[u8]>) -> Option<u32> {
    w.and_then(|w| core::str::from_utf8(w).ok()).and_then(|t| t.parse().ok())
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Media bench");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args).to_vec();
    let words: Vec<&[u8]> = given.split(|b| *b == b' ').filter(|w| !w.is_empty()).collect();
    let level = if words.iter().any(|w| *w == b"--sse") { bench::Simd::Sse }
        else if words.iter().any(|w| *w == b"--avx2") { bench::Simd::Avx2 }
        else { bench::Simd::All };
    let encode = words.iter().any(|w| *w == b"--encode");
    let at = |flag: &[u8]| words.iter().position(|w| *w == flag).map(|i| i + 1);
    let frames = at(b"--frames").and_then(|i| number(words.get(i))).unwrap_or(30).max(1);
    let (w, h) = at(b"--size")
        .and_then(|i| words.get(i))
        .and_then(|s| {
            let mut p = s.split(|b| *b == b'x');
            Some((number(p.next().as_ref())?, number(p.next().as_ref())?))
        })
        .unwrap_or((640, 360));
    let skip: Vec<usize> = [at(b"--frames"), at(b"--size")].into_iter().flatten().collect();
    let paths: Vec<&[u8]> = words.iter().enumerate()
        .filter(|(i, w)| !w.starts_with(b"--") && !skip.contains(i))
        .map(|(_, w)| *w)
        .collect();

    bench::simd(level);
    let mode = match level { bench::Simd::Sse => "sse", bench::Simd::Avx2 => "avx2", bench::Simd::All => "full" };
    let flags = bench::cpu_flags();
    use bench::flag::*;
    sex_pdx::serial_record!("[mediabench.cpu] simd={} flags={:#x} avx={} avx2={} fma3={} avx512={} xcr0={:#x}",
        mode, flags, (flags & AVX != 0) as u8, (flags & AVX2 != 0) as u8, (flags & FMA3 != 0) as u8,
        (flags & AVX512 != 0) as u8, xcr0());
    let mut text = alloc::string::String::new();
    let _ = writeln!(text, "Media bench ({mode}). Results go to the serial log.");
    window.show(text.as_bytes());

    let (mut decoded, mut encoded, mut failed) = (0, 0, 0);
    for path in &paths {
        let name = core::str::from_utf8(path).unwrap_or("?");
        let data = match read_all(&mut window, path) {
            Ok(d) => d,
            Err(why) => {
                sex_pdx::serial_record!("[mediabench.fail] file={} why={}", name, why);
                failed += 1;
                continue;
            }
        };
        let t0 = sex_pdx::get_ticks();
        match bench::decode(&data, Some((1280, 720))) {
            Ok(r) => {
                let ms = sex_pdx::get_ticks() - t0;
                sex_pdx::serial_record!("[mediabench.decode] file={} codec={} simd={} frames={} w={} h={} bytes={} cycles={} convert_cycles={} ms={} hash={:016x}",
                    name, r.codec(), mode, r.frames, r.width, r.height, r.bytes, r.cycles, r.convert_cycles, ms, r.hash);
                let _ = writeln!(text, "{name}: {} frames of {} in {ms} ms", r.frames, r.codec());
                decoded += 1;
            }
            Err(e) => {
                sex_pdx::serial_record!("[mediabench.fail] file={} error={}", name, e);
                failed += 1;
            }
        }
        window.show(text.as_bytes());
    }
    if encode {
        for codec in ["flac", "libx264", "libvpx-vp9", "libaom-av1"] {
            // Sound "frames" are 2 s each; a few are plenty.
            let n = if codec == "flac" { frames.min(10) } else { frames };
            let t0 = sex_pdx::get_ticks();
            match bench::encode(codec, w, h, n) {
                Ok(r) => {
                    let ms = sex_pdx::get_ticks() - t0;
                    sex_pdx::serial_record!("[mediabench.encode] codec={} simd={} frames={} w={} h={} bytes={} cycles={} ms={} hash={:016x}",
                        r.codec(), mode, r.frames, r.width, r.height, r.bytes, r.cycles, ms, r.hash);
                    let _ = writeln!(text, "{codec}: {} frames encoded in {ms} ms", r.frames);
                    encoded += 1;
                }
                // Not in this FFmpeg build (the export profile has the video encoders).
                Err(AVERROR_ENCODER_NOT_FOUND) => sex_pdx::serial_record!("[mediabench.skip] codec={} absent", codec),
                Err(e) => {
                    sex_pdx::serial_record!("[mediabench.fail] codec={} error={}", codec, e);
                    failed += 1;
                }
            }
            window.show(text.as_bytes());
        }
    }
    sex_pdx::serial_record!("[mediabench.done] simd={} decoded={} encoded={} failed={}", mode, decoded, encoded, failed);
    loop {
        if let Event::Close = window.next() { window.close() }
    }
}
