// SPDX-License-Identifier: GPL-3.0-only
//! Syscall and IPC fuzzer: `run fuzz [CALLS] [SEED]`. Makes CALLS kernel calls (default
//! 20000) with numbers and arguments from a seeded generator: every syscall number up to
//! 100 but the few that end, block or reshape this program itself, and pdx calls to any
//! slot with any operation. Arguments mix edge values, kernel and unmapped addresses,
//! pointers into this program's own buffer, and plain noise. Nothing it sends may stop
//! the kernel or any service; the kernel's answer to a bad call is an error, and this
//! program keeps running to say how many calls it made. The same seed makes the same
//! calls, so a failure found once can be found again.
//!
//! `run fuzz CALLS SEED proto` is protocol mode: three calls in four are pdx calls to a
//! service this program holds, with an operation number every `sex-pdx` protocol really
//! uses (collected at build time, see build.rs) and mutated arguments, so each service's
//! request parsing sees well-formed operations with bad arguments, not only noise.
#![no_std]
#![no_main]

use core::fmt::Write as _;
use sex_app::{Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[fuzz.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// Syscalls this program leaves out: 53 waits for a message that never comes, 62 ends
/// the caller, 65 and 80 end the caller's own run, 81-83 change its own memory, code
/// and threads (each has its own test), so the run would end before it reports.
/// 100 is PDX_CALL_WAIT: a valid send may park forever without a reply.
const SKIP: [u64; 8] = [53, 62, 65, 80, 81, 82, 83, 100];

include!(concat!(env!("OUT_DIR"), "/ops.rs"));

/// Answers seen, by value, to report how calls were refused.
struct Answers { values: [u64; 32], counts: [u64; 32], other: u64 }
impl Answers {
    fn note(&mut self, v: u64) {
        for i in 0..32 {
            if self.counts[i] != 0 && self.values[i] == v { self.counts[i] += 1; return; }
            if self.counts[i] == 0 { self.values[i] = v; self.counts[i] = 1; return; }
        }
        self.other += 1;
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*: small, and the same on every machine.
        self.0 ^= self.0 >> 12; self.0 ^= self.0 << 25; self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 { self.next() % n }
}

static mut SCRATCH: [u8; 8192] = [0; 8192];

fn argument(rng: &mut Rng) -> u64 {
    let own = core::ptr::addr_of_mut!(SCRATCH) as u64;
    match rng.below(12) {
        0 => 0,
        1 => 1,
        2 => u64::MAX,
        3 => rng.below(64),
        4 => rng.below(1024),
        5 => own + rng.below(8192),
        6 => own + 8192 - rng.below(16), // straddles the end of our buffer
        7 => 0xffff_8000_0000_0000 + (rng.next() & 0x0000_7fff_ffff_f000), // kernel half
        8 => 0x0000_7fff_ffff_f000 + rng.below(0x2000), // the canonical edge
        9 => 0x4000_0000 + rng.below(16) * 0x0100_0000, // other domains' code slots
        10 => 1u64 << rng.below(64),
        _ => rng.next(),
    }
}

/// One raw syscall with every argument register set; returns rax.
fn call(n: u64, a: [u64; 5]) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") n => ret, inlateout("rdi") a[0] => _, inlateout("rsi") a[1] => _,
            inlateout("rdx") a[2] => _, inlateout("r10") a[3] => _, inlateout("r8") a[4] => _,
            out("rcx") _, out("r11") _, out("r9") _, options(nostack));
    }
    ret
}

fn parse(word: Option<&[u8]>, default: u64) -> u64 {
    word.and_then(|w| core::str::from_utf8(w).ok()).and_then(|t| t.parse().ok()).unwrap_or(default)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Fuzz");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let args = sex_app::files::args(&mut window, &mut args);
    let mut words = args.split(|&b| b == b' ').filter(|w| !w.is_empty());
    let first = words.next();
    if first == Some(&b"iso"[..]) { isolation(window, words.next(), words.next()); }
    let calls = parse(first, 20_000).min(10_000_000);
    let seed = parse(words.next(), 1).max(1);
    let proto = words.next() == Some(&b"proto"[..]);
    sex_pdx::serial_record!("[fuzz.start] target={} task={} calls={} seed={} proto={} ops={}", window.target, window.task, calls, seed, proto as u8, OPS.len());
    let mut answers = Answers { values: [0; 32], counts: [0; 32], other: 0 };
    let mut ops_sent = [0u8; 1024];
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (mut made, mut ipc, mut ok) = (0u64, 0u64, 0u64);
    while made < calls {
        const HELD: [u64; 7] = [sex_pdx::SLOT_STORAGE, sex_pdx::SLOT_DISPLAY, sex_pdx::SLOT_SHELL,
            sex_pdx::SLOT_BELL, sex_pdx::SLOT_NET, sex_pdx::fs::CLIENT_SLOT, 0];
        if proto && rng.below(4) != 0 {
            // A real operation to a held service, with arguments that are edge values,
            // pointers or noise. The send's answer is whatever the service replies.
            let k = rng.below(OPS.len() as u64) as usize;
            let a = [HELD[rng.below(HELD.len() as u64) as usize], OPS[k].1,
                argument(&mut rng), argument(&mut rng), argument(&mut rng)];
            let r = call(0, a);
            answers.note(r);
            if r == 0 { ok += 1; }
            if k < ops_sent.len() * 8 { ops_sent[k / 8] |= 1 << (k % 8); }
            ipc += 1; made += 1;
            if made % 5000 == 0 {
                sex_pdx::serial_record!("[fuzz.progress] calls={} ipc={} ok={}", made, ipc, ok);
                if let Some(Event::Close) = window.poll() { window.close() }
            }
            continue;
        }
        let n = if rng.below(4) == 0 { 0 } else { rng.below(101) };
        if SKIP.contains(&n) { continue; }
        let mut a = [argument(&mut rng), argument(&mut rng), argument(&mut rng), argument(&mut rng), argument(&mut rng)];
        match n {
            // pdx_call: a slot this program may or may not hold, any operation.
            // Half the time a slot a program holds (Storage, Display, Shell, Bell, Net, the
            // file region's route), so services see the bad requests, not only the kernel.
            0 => {
                a[0] = if rng.below(2) == 0 { HELD[rng.below(HELD.len() as u64) as usize] } else { rng.below(600) };
                if rng.below(2) == 0 { a[1] = rng.below(0x400); }
                ipc += 1;
            }
            // A bounded wait, kept short.
            70 => a[0] = rng.below(3),
            _ => {}
        }
        let r = call(n, a);
        answers.note(r);
        if r == 0 { ok += 1; }
        made += 1;
        if made % 5000 == 0 {
            sex_pdx::serial_record!("[fuzz.progress] calls={} ipc={} ok={}", made, ipc, ok);
            // Answer Close and let Shell's and Display's own work through between bursts.
            if let Some(Event::Close) = window.poll() { window.close() }
        }
    }
    let covered: u32 = ops_sent.iter().map(|b| b.count_ones()).sum();
    for i in 0..32 {
        if answers.counts[i] == 0 { break; }
        sex_pdx::serial_record!("[fuzz.answer] value={:#x} count={}", answers.values[i], answers.counts[i]);
    }
    sex_pdx::serial_record!("[fuzz.ops] covered={} of={} other_answers={}", covered, OPS.len(), answers.other);
    sex_pdx::serial_record!("[fuzz.done] target={} task={} calls={} ipc={} ok={} seed={}", window.target, window.task, made, ipc, ok, seed);
    let mut text = Text { buf: [0; 256], len: 0 };
    let _ = write!(text, "Fuzz run done.\n{} calls ({} pdx calls), {} answered OK.\nSeed {}. F11 closes.", made, ipc, ok, seed);
    window.show(text.as_bytes());
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[fuzz.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}

/// Isolation matrix probes (scripts/isolation_matrix_gate.py, kernel plan phase 7).
/// `run fuzz iso hold` stays open and names addresses in its own memory: a code
/// address (`iso_target_code`, which only returns), a data page and a stack slot.
/// `run fuzz iso OP ADDR` (OP = read, write, exec; ADDR hex) touches that address once.
/// The gate passes a hold's addresses to a second run, which must then be ended by
/// the kernel; `[fuzz.iso.breach]` means the touch went through. `own` proves the
/// probe itself works on this run's own memory.
fn isolation(mut window: Window, op: Option<&[u8]>, addr: Option<&[u8]>) -> ! {
    let hex = |w: Option<&[u8]>| w.and_then(|w| core::str::from_utf8(w).ok())
        .and_then(|t| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok());
    // Keeps the marker the gate patches into WRPKRU in this file's code.
    core::hint::black_box(iso_key_marker as *const () as usize);
    match (op, hex(addr)) {
        (Some(b"hold"), _) => {
            let stack_slot = core::hint::black_box([0x5Au8; 64]);
            let data = core::ptr::addr_of_mut!(ISO_DATA) as u64;
            unsafe { core::ptr::write_volatile(data as *mut u8, 0x5A); }
            sex_pdx::serial_record!("[fuzz.iso.hold] target={} task={} code={:#x} data={:#x} stack={:#x}",
                window.target, window.task, iso_target_code as *const () as usize, data, stack_slot.as_ptr() as u64);
            window.show(b"Isolation probe: holding. F11 closes.");
            loop {
                if let Event::Close = window.next() {
                    let kept = unsafe { core::ptr::read_volatile(data as *const u8) };
                    sex_pdx::serial_record!("[fuzz.iso.closed] target={} task={} data_byte={:#x}", window.target, window.task, kept);
                    core::hint::black_box(stack_slot);
                    window.close()
                }
            }
        }
        (Some(b"own"), _) => {
            let data = core::ptr::addr_of_mut!(ISO_DATA) as u64;
            unsafe { core::ptr::write_volatile(data as *mut u8, 0x33); }
            let back = unsafe { core::ptr::read_volatile(data as *const u8) };
            iso_target_code();
            sex_pdx::serial_record!("[fuzz.iso.own] target={} task={} ok={}", window.target, window.task, (back == 0x33) as u8);
        }
        (Some(op @ (b"read" | b"write" | b"exec")), Some(addr)) => {
            let name = core::str::from_utf8(op).unwrap_or("?");
            sex_pdx::serial_record!("[fuzz.iso.try] target={} task={} op={} addr={:#x}", window.target, window.task, name, addr);
            let seen = unsafe {
                match op {
                    b"read" => core::ptr::read_volatile(addr as *const u8) as u64,
                    b"write" => { core::ptr::write_volatile(addr as *mut u8, 0xA5); 0xA5 }
                    _ => { let f: extern "C" fn() = core::mem::transmute(addr as usize); f(); 0 }
                }
            };
            sex_pdx::serial_record!("[fuzz.iso.breach] target={} task={} op={} addr={:#x} byte={:#x}", window.target, window.task, name, addr, seen);
        }
        _ => sex_pdx::serial_record!("[fuzz.iso.usage] target={} task={}", window.target, window.task),
    }
    window.show(b"Isolation probe done. F11 closes.");
    loop {
        if let Event::Close = window.next() { window.close() }
    }
}

static mut ISO_DATA: [u8; 4096] = [0; 4096];

/// Code another run may try to execute: it only returns.
#[inline(never)]
#[no_mangle]
extern "C" fn iso_target_code() { core::hint::black_box(()); }

/// Never called. Its body is one 7-byte NOP whose operand spells "KEYH"
/// (0F 1F 80 4B 45 59 48); the gate overwrites it with WRPKRU (0F 01 EF) plus a
/// 4-byte NOP in a copy of this file, and the loader must refuse that copy.
#[inline(never)]
#[no_mangle]
extern "C" fn iso_key_marker() {
    unsafe { core::arch::asm!(".byte 0x0f, 0x1f, 0x80, 0x4b, 0x45, 0x59, 0x48", options(nomem, nostack, preserves_flags)); }
}

struct Text { buf: [u8; 256], len: usize }
impl Text { fn as_bytes(&self) -> &[u8] { &self.buf[..self.len] } }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
