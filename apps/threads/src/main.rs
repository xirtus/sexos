// SPDX-License-Identifier: GPL-3.0-only
//! Threads: one program on several cores. `run threads` draws a Mandelbrot set twice,
//! once on one task and once split into bands across workers on other cores
//! (`sex_app::thread::parallel`), checks both pictures match, and shows the picture
//! and how much faster the split one was. Each task also checks it has its own
//! `#[thread_local]` statics.
//!
//! Keys, each for its gate: S starts workers that spin forever (Close must still end
//! the program and every worker); P makes a worker panic, which ends the whole program.
#![no_std]
#![no_main]
#![feature(thread_local)]

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

/// The last timed wait: ticks waited << 8 | status, printed if the check fails.
static TIMED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SPIN: AtomicBool = AtomicBool::new(true);

/// Each task's own: one with a starting value (.tdata), one zeroed (.tbss).
#[thread_local]
static mut SEEDED: u64 = 7;
#[thread_local]
static mut ZEROED: [u64; 4] = [0; 4];

/// C threads: four pthreads add to one counter under a mutex, a condition variable
/// hands a value across, a key's destructor runs at each thread's end, `pthread_once`
/// runs once, and `pthread_join` returns each result. Returns what went wrong, if anything.
fn pthread_check() -> Result<u64, &'static str> {
    use core::ffi::c_void;
    use sex_libc::pthread::*;
    static mut LOCK: pthread_mutex_t = unsafe { core::mem::zeroed() };
    static mut COND: pthread_cond_t = unsafe { core::mem::zeroed() };
    static mut ONCE: pthread_once_t = 0;
    static mut KEY: pthread_key_t = 0;
    static mut COUNT: u64 = 0;
    static mut READY: u64 = 0;
    static ONCES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    static DTORS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    unsafe extern "C" fn first() { ONCES.fetch_add(1, Ordering::Relaxed); }
    unsafe extern "C" fn dtor(_: *mut c_void) { DTORS.fetch_add(1, Ordering::Relaxed); }
    unsafe extern "C" fn work(arg: *mut c_void) -> *mut c_void {
        unsafe {
            pthread_once(core::ptr::addr_of_mut!(ONCE), first);
            pthread_setspecific(KEY, 1 as *const c_void);
            for _ in 0..2000 {
                pthread_mutex_lock(core::ptr::addr_of_mut!(LOCK));
                let c = core::ptr::addr_of_mut!(COUNT);
                c.write_volatile(c.read_volatile() + 1);
                pthread_mutex_unlock(core::ptr::addr_of_mut!(LOCK));
            }
            // The last one to finish counting tells the main task.
            pthread_mutex_lock(core::ptr::addr_of_mut!(LOCK));
            READY += 1;
            pthread_cond_broadcast(core::ptr::addr_of_mut!(COND));
            pthread_mutex_unlock(core::ptr::addr_of_mut!(LOCK));
        }
        (arg as usize * 10) as *mut c_void
    }
    unsafe {
        if pthread_key_create(core::ptr::addr_of_mut!(KEY), Some(dtor)) != 0 { return Err("key"); }
        let mut threads = [core::ptr::null_mut(); 4];
        for (i, th) in threads.iter_mut().enumerate() {
            if pthread_create(th, core::ptr::null(), work, (i + 1) as *mut c_void) != 0 { return Err("create"); }
        }
        pthread_mutex_lock(core::ptr::addr_of_mut!(LOCK));
        while core::ptr::addr_of!(READY).read_volatile() < 4 { pthread_cond_wait(core::ptr::addr_of_mut!(COND), core::ptr::addr_of_mut!(LOCK)); }
        pthread_mutex_unlock(core::ptr::addr_of_mut!(LOCK));
        for (i, th) in threads.iter().enumerate() {
            let mut out = core::ptr::null_mut();
            if pthread_join(*th, &mut out) != 0 || out as usize != (i + 1) * 10 { return Err("join"); }
        }
        if COUNT != 8000 { return Err("count"); }
        // Nobody signals: a timed wait must give up on time, sleeping in the kernel.
        sex_libc::time::set_clock(|| { let ns = sex_pdx::get_ticks() * 1_000_000; (ns as i64, ns) });
        let mut attr: pthread_condattr_t = core::mem::zeroed();
        pthread_condattr_init(&mut attr);
        pthread_condattr_setclock(&mut attr, 1);
        let mut quiet: pthread_cond_t = core::mem::zeroed();
        pthread_cond_init(&mut quiet, &attr);
        // The start is read before the clock, so a tick between the two cannot make
        // an on-time wake look one tick early.
        let start = sex_pdx::get_ticks();
        let mut now = sex_libc::time::timespec { tv_sec: 0, tv_nsec: 0 };
        sex_libc::time::clock_gettime(1, &mut now);
        let mut until = sex_libc::time::timespec { tv_sec: now.tv_sec, tv_nsec: now.tv_nsec + 150_000_000 };
        if until.tv_nsec >= 1_000_000_000 { until.tv_sec += 1; until.tv_nsec -= 1_000_000_000; }
        pthread_mutex_lock(core::ptr::addr_of_mut!(LOCK));
        let timed = pthread_cond_timedwait(&mut quiet, core::ptr::addr_of_mut!(LOCK), &until);
        pthread_mutex_unlock(core::ptr::addr_of_mut!(LOCK));
        let waited = sex_pdx::get_ticks() - start;
        crate::TIMED.store((waited << 8) | (timed as u64 & 0xFF), Ordering::Relaxed);
        if timed != ETIMEDOUT || !(150..2000).contains(&waited) { return Err("timed wait"); }
        if ONCES.load(Ordering::Relaxed) != 1 { return Err("once"); }
        if DTORS.load(Ordering::Relaxed) != 4 { return Err("destructor"); }
        Ok(COUNT)
    }
}

/// Every task adds to its own thread-locals and must see only its own sums.
fn tls_check(bands: usize) -> usize {
    use core::sync::atomic::AtomicUsize;
    let good = AtomicUsize::new(0);
    thread::parallel(bands, &|i| {
        for _ in 0..1000 {
            unsafe {
                let s = core::ptr::addr_of_mut!(SEEDED);
                s.write_volatile(s.read_volatile() + i as u64);
                let z = core::ptr::addr_of_mut!(ZEROED);
                (*z)[i % 4] = (*z)[i % 4] + 1;
            }
            sex_pdx::sys_yield();
        }
        let (s, z) = unsafe { (core::ptr::addr_of!(SEEDED).read_volatile(), (*core::ptr::addr_of!(ZEROED))[i % 4]) };
        if s == 7 + 1000 * i as u64 && z == 1000 { good.fetch_add(1, Ordering::Relaxed); }
    });
    good.load(Ordering::Relaxed)
}

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
    let good = tls_check(bands);
    if good == bands {
        sex_pdx::serial_record!("[threads.tls] target={} task={} tasks={} pointer={:#x}", target, task, bands, sex_app::tls::pointer());
    } else {
        sex_pdx::serial_record!("[threads.fail] target={} task={} why=thread-locals shared good={} of {}", target, task, good, bands);
    }
    match pthread_check() {
        Ok(count) => {
            sex_pdx::serial_record!("[threads.pthread] target={} task={} threads=4 count={}", target, task, count);
            sex_pdx::serial_record!("[threads.timed] target={} task={} waited_ticks={}", target, task, TIMED.load(Ordering::Relaxed) >> 8);
        }
        Err(why) => {
            let timed = TIMED.load(Ordering::Relaxed);
            sex_pdx::serial_record!("[threads.fail] target={} task={} why=pthread {} waited_ticks={} status={}", target, task, why, timed >> 8, timed & 0xFF)
        }
    }
    // A worker uses Files too: create, write, stat and read a file, then the main task
    // checks what it saw. A worker's call must not sleep waiting for a reply that only
    // wakes the main task.
    static FILE_STEP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    if let Ok(h) = thread::spawn(|| {
        use sex_app::files::direct;
        let step = |n: u32| FILE_STEP.store(n, Ordering::Release);
        let _ = direct::create(b"worker.txt");
        step(1); if direct::write_at(b"worker.txt", 0, b"from a worker") != Ok(13) { return; }
        step(2); if !direct::stat(b"worker.txt").is_ok_and(|st| st.size == 13) { return; }
        let mut back = [0u8; 13];
        step(3); if direct::read_at(b"worker.txt", 0, &mut back) != Ok(13) || &back != b"from a worker" { return; }
        step(4);
    }) { h.join(); }
    match FILE_STEP.load(Ordering::Acquire) {
        4 => sex_pdx::serial_record!("[threads.files] target={} task={} worker=1 ok=1", target, task),
        n => sex_pdx::serial_record!("[threads.fail] target={} task={} why=worker file request failed at step {}", target, task, n),
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
