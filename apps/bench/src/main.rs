// SPDX-License-Identifier: GPL-3.0-only
//! Bench: times the operations real programs lean on, so SexOS can be measured against
//! itself over time and against Linux on the same machine (tools/bench/linux holds the
//! same tests as a Linux program). `run bench`. Each result is cycles per operation
//! from the time-stamp counter, written to serial as `[bench] name=... cycles=...`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use core::fmt::Write as _;
use sex_app::{files, Event, Window};
use sex_pdx::program_memory as memory;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[bench.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn tsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

struct Report { text: alloc::string::String }
impl Report {
    fn add(&mut self, name: &str, ops: u64, cycles: u64, unit: &str) {
        let per = cycles / ops.max(1);
        sex_pdx::serial_record!("[bench] name={} ops={} cycles={} per={}", name, ops, per, unit);
        let _ = writeln!(self.text, "{:<14} {:>12} cycles per {}", name, per, unit);
    }
}

const MIB: usize = 1024 * 1024;
const FILE: &[u8] = b"bench.dat";

fn run(window: &mut Window, r: &mut Report) -> Result<(), &'static str> {
    // A system call that does nothing (40 only answers 0): the cost of entering the
    // kernel. Then one that does a little: asking how much memory the run holds.
    let n = 20_000;
    let t = tsc();
    for _ in 0..n { unsafe { core::arch::asm!("syscall", inlateout("rax") 40u64 => _, out("rcx") _, out("r11") _, options(nostack)); } }
    r.add("null_syscall", n, tsc() - t, "call");
    let t = tsc();
    for _ in 0..n { let _ = memory::grow(0); }
    r.add("memory_query", n, tsc() - t, "call");

    // Giving the processor away and getting it back.
    let n = 2_000;
    let t = tsc();
    for _ in 0..n { sex_pdx::sys_yield(); }
    r.add("yield", n, tsc() - t, "yield");

    // Memory: allocate and touch 32 MiB, then copy 8 MiB a few times.
    let t = tsc();
    let mut big: Vec<u8> = Vec::new();
    big.try_reserve_exact(32 * MIB).map_err(|_| "no 32 MiB")?;
    big.resize(32 * MIB, 1);
    r.add("alloc_touch", 32, tsc() - t, "MiB");
    let mut copy: Vec<u8> = alloc::vec![0u8; 8 * MIB];
    let t = tsc();
    for i in 0..4 { copy.copy_from_slice(&big[i * MIB..i * MIB + 8 * MIB]); }
    r.add("memcpy", 32, tsc() - t, "MiB");
    if copy[0] != 1 { return Err("copy lost data"); }
    drop(copy);

    // Files: write 4 MiB, make it durable, read it back, and ask its size many times.
    let data = &big[..4 * MIB];
    let t = tsc();
    files::write(window, FILE, data).map_err(|_| "write failed")?;
    files::sync(window).map_err(|_| "sync failed")?;
    r.add("file_write", 4, tsc() - t, "MiB");
    let mut back: Vec<u8> = alloc::vec![0u8; 4 * MIB];
    let t = tsc();
    let got = files::read(window, FILE, &mut back).map_err(|_| "read failed")?;
    r.add("file_read", 4, tsc() - t, "MiB");
    if got != 4 * MIB || back != data { return Err("file came back different"); }
    let n = 500;
    let t = tsc();
    for _ in 0..n { if files::size(window, FILE) != Ok(4 * MIB as u64) { return Err("wrong size"); } }
    r.add("file_stat", n, tsc() - t, "call");
    Ok(())
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Bench");
    let mut report = Report { text: alloc::string::String::new() };
    let _ = writeln!(report.text, "Bench. F11 closes.\n");
    match run(&mut window, &mut report) {
        Ok(()) => sex_pdx::serial_record!("[bench.done] target={} task={}", window.target, window.task),
        Err(why) => {
            sex_pdx::serial_record!("[bench.fail] target={} task={} why={}", window.target, window.task, why);
            let _ = writeln!(report.text, "\nFailed: {}", why);
        }
    }
    window.show(report.text.as_bytes());
    loop { if let Event::Close = window.next() { window.close() } }
}
