// SPDX-License-Identifier: GPL-3.0-only
//! JIT: a program that compiles while it runs, the way a JavaScript engine does.
//! `run jit`. It compiles a small expression to x86-64 machine code in its own heap,
//! asks the kernel to seal those pages as code (`sex_app::code`), and runs it against
//! an interpreter for the same expression, so both answers and their speeds show.
//!
//! It also shows the rules the kernel keeps:
//! - sealed code is executable but not writable, and data is writable but not
//!   executable, so no page is both;
//! - code holding an instruction that could change the program's protection keys
//!   (WRPKRU) is refused, even hidden inside a constant, and the compiler re-encodes it;
//! - code can be opened, patched and sealed again.
//!
//! Keys, each for its gate: W writes to sealed code and E runs open data. Each must end
//! only this run, through a page fault.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use sex_app::code::{CodeBuf, Error};
use sex_app::{Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[jit.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// A tiny stack language: what a parser would make of an expression in x.
#[derive(Clone, Copy)]
enum Op { X, Const(i64), Add, Sub, Mul }

/// f(x) = ((x*x - 3*x) * x + 7*x) - K, written as stack operations.
fn program(k: i64) -> [Op; 15] {
    use Op::*;
    [X, X, Mul, Const(3), X, Mul, Sub, X, Mul, Const(7), X, Mul, Add, Const(k), Sub]
}

/// The interpreter: what a JS engine without a JIT does for every operation.
fn interpret(ops: &[Op], x: i64) -> i64 {
    let mut stack = [0i64; 16];
    let mut n = 0;
    for op in ops {
        match *op {
            Op::X => { stack[n] = x; n += 1 }
            Op::Const(c) => { stack[n] = c; n += 1 }
            Op::Add | Op::Sub | Op::Mul => {
                let (a, b) = (stack[n - 2], stack[n - 1]);
                n -= 1;
                stack[n - 1] = match *op { Op::Add => a.wrapping_add(b), Op::Sub => a.wrapping_sub(b), _ => a.wrapping_mul(b) };
            }
        }
    }
    stack[0]
}

/// The compiler: one machine-code sequence per operation, on the machine stack.
/// The function is `extern "sysv64" fn(x: i64) -> i64`; x arrives in rdi.
/// `split` re-encodes every constant as two halves, for when sealing refused a
/// constant whose bytes happen to spell a forbidden instruction.
fn compile(ops: &[Op], split: bool) -> Vec<u8> {
    let mut c = Vec::new();
    for op in ops {
        match *op {
            Op::X => c.push(0x57), // push rdi
            Op::Const(v) => {
                if split {
                    // mov rax, v/2 ; mov rcx, v - v/2 ; add rax, rcx
                    let (a, b) = (v / 2, v - v / 2);
                    c.extend_from_slice(&[0x48, 0xB8]); c.extend_from_slice(&a.to_le_bytes());
                    c.extend_from_slice(&[0x48, 0xB9]); c.extend_from_slice(&b.to_le_bytes());
                    c.extend_from_slice(&[0x48, 0x01, 0xC8]);
                } else {
                    c.extend_from_slice(&[0x48, 0xB8]); c.extend_from_slice(&v.to_le_bytes()); // mov rax, v
                }
                c.push(0x50); // push rax
            }
            Op::Add | Op::Sub | Op::Mul => {
                c.extend_from_slice(&[0x59, 0x58]); // pop rcx ; pop rax
                match *op {
                    Op::Add => c.extend_from_slice(&[0x48, 0x01, 0xC8]),       // add rax, rcx
                    Op::Sub => c.extend_from_slice(&[0x48, 0x29, 0xC8]),       // sub rax, rcx
                    _ => c.extend_from_slice(&[0x48, 0x0F, 0xAF, 0xC1]),       // imul rax, rcx
                }
                c.push(0x50); // push rax
            }
        }
    }
    c.extend_from_slice(&[0x58, 0xC3]); // pop rax ; ret
    c
}

type Jitted = extern "sysv64" fn(i64) -> i64;

fn ticks() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

const N: i64 = 200_000;
/// A constant whose bytes contain 0F 01 EF (WRPKRU) when encoded directly. Made at run
/// time, because the kernel refuses a program file whose own code holds those bytes.
fn trap() -> i64 { core::hint::black_box(0x00EF_0000i64) | core::hint::black_box(0x010F) }

struct Report { text: String, speedup: u64 }

fn run() -> Result<Report, String> {
    let mut text = String::new();
    let _ = writeln!(text, "JIT: machine code made while the program runs.\n");
    let k = 11;
    let ops = program(k);
    let mut buf = CodeBuf::new(4096).map_err(|e| alloc::format!("no code memory: {:?}", e))?;
    buf.load(&compile(&ops, false)).map_err(|e| alloc::format!("seal refused: {:?}", e))?;
    let f: Jitted = unsafe { buf.entry(0) };

    // Same answers, then the time each takes for N values of x.
    for x in [-5, 0, 1, 2, 1000, 123_456_789] {
        let (a, b) = (interpret(&ops, x), f(x));
        if a != b { return Err(alloc::format!("f({}) = {} interpreted but {} compiled", x, a, b)); }
    }
    let t0 = ticks();
    let mut s1 = 0i64;
    for x in 0..N { s1 = s1.wrapping_add(interpret(core::hint::black_box(&ops), x)); }
    let t1 = ticks();
    let mut s2 = 0i64;
    for x in 0..N { s2 = s2.wrapping_add(core::hint::black_box(f)(x)); }
    let t2 = ticks();
    if s1 != s2 { return Err(alloc::format!("sums differ: {} vs {}", s1, s2)); }
    let (interp, jit) = ((t1 - t0).max(1), (t2 - t1).max(1));
    let speedup = interp / jit;
    sex_pdx::serial_record!("[jit.speed] n={} interpreter_cycles={} jit_cycles={} speedup={} sum={}", N, interp, jit, speedup, s1);
    let _ = writeln!(text, "f(x) = (x*x - 3x)*x + 7x - {}, for x = 0..{}:", k, N);
    let _ = writeln!(text, "  interpreter: {} Mcycles\n  compiled:    {} Mcycles  ({}x faster)\n", interp / 1_000_000, jit / 1_000_000, speedup);

    // The kernel refuses a constant that spells WRPKRU; the compiler re-encodes it.
    let trap = program(trap());
    let mut other = CodeBuf::new(4096).map_err(|e| alloc::format!("no code memory: {:?}", e))?;
    match other.load(&compile(&trap, false)) {
        Err(Error::Blocked(at)) => {
            sex_pdx::serial_record!("[jit.blocked] offset={}", at);
            let _ = writeln!(text, "A constant hiding WRPKRU was refused at byte {}.", at);
        }
        Ok(()) => return Err("code hiding WRPKRU was sealed".into()),
        Err(e) => return Err(alloc::format!("unexpected refusal: {:?}", e)),
    }
    if other.is_sealed() { return Err("a refused buffer reports sealed".into()); }
    other.load(&compile(&trap, true)).map_err(|e| alloc::format!("re-encoded code refused: {:?}", e))?;
    let g: Jitted = unsafe { other.entry(0) };
    if g(9) != interpret(&trap, 9) { return Err("re-encoded code is wrong".into()); }
    let _ = writeln!(text, "Re-encoded as two halves, it sealed and runs.\n");

    // Open, patch, seal: a new constant in place, as an inline cache update would.
    buf.load(&compile(&program(-1000), false)).map_err(|e| alloc::format!("patch refused: {:?}", e))?;
    let f: Jitted = unsafe { buf.entry(0) };
    if f(3) != interpret(&program(-1000), 3) { return Err("patched code is wrong".into()); }
    let _ = writeln!(text, "Opened, patched and sealed again: f(3) = {}.", f(3));
    sex_pdx::serial_record!("[jit.patched] f3={}", f(3));
    let _ = writeln!(text, "\nAll checks passed. W writes to code, E runs data (each ends this run). F11 closes.");
    // Kept alive for W and E below.
    core::mem::forget(buf);
    core::mem::forget(other);
    Ok(Report { text, speedup })
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"JIT");
    let (target, task) = (window.target, window.task);
    match run() {
        Ok(report) => {
            sex_pdx::serial_record!("[jit.ok] target={} task={} speedup={}", target, task, report.speedup);
            window.show(report.text.as_bytes());
        }
        Err(why) => {
            sex_pdx::serial_record!("[jit.fail] target={} task={} why={}", target, task, why);
            window.show(why.as_bytes());
        }
    }
    sex_pdx::serial_record!("[jit.ready] target={} task={}", target, task);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[jit.closed] target={} task={}", target, task); window.close() }
            // W: write into sealed code. The page is not writable: this run must end.
            Event::Key(key) if key.code == 0x11 => {
                let mut buf = CodeBuf::new(4096).ok().filter(|b| b.as_ptr() as usize != 0);
                let Some(b) = buf.as_mut() else { continue };
                if b.load(&[0xC3]).is_err() { continue }
                sex_pdx::serial_record!("[jit.write_code] target={} task={}", target, task);
                unsafe { core::ptr::write_volatile(b.as_ptr() as *mut u8, 0x90) };
                sex_pdx::serial_record!("[jit.fail] target={} task={} why=wrote sealed code", target, task);
            }
            // E: run bytes that are still data. The page is not executable: this run must end.
            Event::Key(key) if key.code == 0x12 => {
                let mut data = CodeBuf::new(4096).ok();
                let Some(d) = data.as_mut() else { continue };
                if let Some(bytes) = d.bytes_mut() { bytes[0] = 0xC3; }
                sex_pdx::serial_record!("[jit.run_data] target={} task={}", target, task);
                let f: Jitted = unsafe { core::mem::transmute(d.as_ptr()) };
                core::hint::black_box(f)(0);
                sex_pdx::serial_record!("[jit.fail] target={} task={} why=ran data", target, task);
            }
            _ => {}
        }
    }
}
