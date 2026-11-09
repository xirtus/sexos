// SPDX-License-Identifier: GPL-3.0-only
//! Threads: one program on several cores. `run threads` draws a Mandelbrot set twice,
//! once on one task and once split into bands across workers on other cores
//! (`sex_app::thread::parallel`), checks both pictures match, and shows the picture
//! and how much faster the split one was.
//!
//! Keys, each for its gate: S starts workers that spin forever (Close must still end
//! the program and every worker); P makes a worker panic, which ends the whole program.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec;
use core::sync::atomic::{AtomicBool, Ordering};
use sex_app::{thread, Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // The message alone: the P key's panic is on purpose, and gates treat
    // "panicked at" in the log as a failure.
    sex_pdx::serial_record!("[threads.fatal] {}", info.message());
    sex_pdx::task_finish(1);
}

const W: usize = 480;
const H: usize = 320;
const ITER: u32 = 200;

fn ticks() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

fn colour(n: u32) -> u32 {
    if n >= ITER { return 0x0010_1820; }
    let t = n as f64 / ITER as f64;
    let r = (9.0 * (1.0 - t) * t * t * t * 255.0) as u32;
    let g = (15.0 * (1.0 - t) * (1.0 - t) * t * t * 255.0) as u32;
    let b = (8.5 * (1.0 - t) * (1.0 - t) * (1.0 - t) * t * 255.0) as u32;
    (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

/// Rows `from..to` of the picture into `rows` (which starts at row `from`).
fn draw(rows: &mut [u32], from: usize, to: usize) {
    for y in from..to {
        let ci = (y as f64 / H as f64) * 2.4 - 1.2;
        for x in 0..W {
            let cr = (x as f64 / W as f64) * 3.5 - 2.5;
            let (mut zr, mut zi, mut n) = (0.0f64, 0.0f64, 0);
            while n < ITER && zr * zr + zi * zi <= 4.0 {
                let t = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = t;
                n += 1;
            }
            rows[(y - from) * W + x] = colour(n);
        }
    }
}

fn hash(p: &[u32]) -> u64 { p.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ *v as u64).wrapping_mul(0x1_0000_0001_b3)) }

/// A raw pointer that may cross to a worker: each band writes only its own rows.
#[derive(Clone, Copy)]
struct Rows(*mut u32);
unsafe impl Send for Rows {}
unsafe impl Sync for Rows {}

fn split(out: &mut [u32], bands: usize) {
    let base = Rows(out.as_mut_ptr());
    let per = H.div_ceil(bands);
    thread::parallel(bands, &move |i| {
        let base = base;
        let (from, to) = ((i * per).min(H), ((i + 1) * per).min(H));
        let rows = unsafe { core::slice::from_raw_parts_mut(base.0.add(from * W), (to - from) * W) };
        draw(rows, from, to);
    });
}

static SPIN: AtomicBool = AtomicBool::new(true);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Threads");
    let (target, task) = (window.target, window.task);
    let cores = thread::cores();
    let bands = cores.max(2);
    let mut one = vec![0u32; W * H];
    let t0 = ticks();
    draw(&mut one, 0, H);
    let t1 = ticks();
    let mut many = vec![0u32; W * H];
    split(&mut many, bands);
    let t2 = ticks();
    let (single, multi) = ((t1 - t0).max(1), (t2 - t1).max(1));
    let speedup_x100 = single * 100 / multi;
    let same = hash(&one) == hash(&many);
    if same {
        sex_pdx::serial_record!("[threads.ok] target={} task={} cores={} bands={} single_cycles={} multi_cycles={} speedup_x100={} hash={:#x}",
            target, task, cores, bands, single, multi, speedup_x100, hash(&many));
    } else {
        sex_pdx::serial_record!("[threads.fail] target={} task={} why=pictures differ", target, task);
    }
    let pixels = window.pixels();
    pixels[..W * H].copy_from_slice(&many);
    window.show_pixels(W, H);
    let mut title = *b"Threads x.xx    ";
    let s = speedup_x100.min(999);
    title[8] = b'0' + (s / 100) as u8; title[10] = b'0' + (s / 10 % 10) as u8; title[11] = b'0' + (s % 10) as u8;
    window.title(&title);
    sex_pdx::serial_record!("[threads.ready] target={} task={}", target, task);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[threads.closed] target={} task={}", target, task); window.close() }
            // S: workers that never finish. Close must still end them all.
            Event::Key(key) if key.code == 0x1F => {
                let mut started = 0;
                for _ in 0..bands {
                    if let Ok(h) = thread::spawn(|| { while SPIN.load(Ordering::Relaxed) { core::hint::spin_loop(); } }) {
                        core::mem::forget(h);
                        started += 1;
                    }
                }
                sex_pdx::serial_record!("[threads.spinning] target={} task={} workers={}", target, task, started);
            }
            // P: a worker panics; the whole program must end.
            Event::Key(key) if key.code == 0x19 => {
                sex_pdx::serial_record!("[threads.panic] target={} task={}", target, task);
                thread::parallel(2, &|i| if i == 1 { panic!("worker panic on purpose") });
                sex_pdx::serial_record!("[threads.fail] target={} task={} why=survived a worker panic", target, task);
            }
            _ => {}
        }
    }
}
