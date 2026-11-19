// SPDX-License-Identifier: GPL-3.0-only
//! Crossdress (CW1): runs a 64-bit Windows program in a Drag cell. `run crossdress` runs
//! the built-in test program (`crates/sex-crossdress/test/hello.c`); `run crossdress
//! FILE.exe` runs an .exe from this program's folder (or a file it was given).
//!
//! The cell is a SYS_VM guest with no guest OS (`sex_crossdress`). Its memory is this
//! run's own heap, lent to the VM. The Windows code never runs in SexOS's address space,
//! so it never meets the WRPKRU byte check and can never touch a host protection key.
//!
//! Serial log: `[crossdress.start]`, `[crossdress.loaded] base=... entry=... imports=...`,
//! then either `[crossdress.refused] status=3 reason=...` (no usable VMX: QEMU TCG, AMD
//! before Drag D5) or `[crossdress.run]`, one `[crossdress.out] ...` per line the program
//! writes, and `[crossdress.stop] reason=... code=... exits=... calls=... exceptions=...
//! handled=...`. Failures: `[crossdress.fail]`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use drag_vmm::memory::{self, Ram};
use drag_vmm::vmm::Vcpu;
use sex_app::{files, Event, Window};
use sex_crossdress::process::{Host, WinProcess, Stop};
use sex_pdx::vm::{self, Exit, State};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[crossdress.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// The cell's memory. 64 MiB covers the test program and small tools; bigger programs
/// will size it from the image (CW4).
const CELL_MIB: usize = 64;
const LOAN_PAGES: u64 = 4096;

/// FXSAVE areas: the guest's x87/SSE state is the CPU's while it runs (as in Drag).
#[repr(C, align(16))]
struct Fx([u8; 512]);

struct KernelVcpu { vm: u64, cpu: u64, host_fx: Fx, guest_fx: Fx }

impl KernelVcpu {
    fn new(vm: u64, cpu: u64) -> KernelVcpu {
        let mut guest = Fx([0; 512]);
        guest.0[0..2].copy_from_slice(&0x037Fu16.to_le_bytes()); // FCW at reset
        guest.0[24..28].copy_from_slice(&0x1F80u32.to_le_bytes()); // MXCSR at reset
        KernelVcpu { vm, cpu, host_fx: Fx([0; 512]), guest_fx: guest }
    }
}

impl Vcpu for KernelVcpu {
    fn run(&mut self, exit: &mut Exit, load_regs: bool) -> Result<(), u64> {
        let flags = if load_regs { vm::RUN_LOAD_REGS } else { 0 };
        let status: u64;
        // One asm block, so no compiler-generated SSE code runs between the swaps.
        unsafe {
            core::arch::asm!(
                "fxsave64 [{host}]",
                "fxrstor64 [{guest}]",
                "syscall",
                "fxsave64 [{guest}]",
                "fxrstor64 [{host}]",
                host = in(reg) self.host_fx.0.as_mut_ptr(),
                guest = in(reg) self.guest_fx.0.as_mut_ptr(),
                inlateout("rax") vm::SYS_VM => status,
                in("rdi") vm::OP_VCPU_RUN,
                inlateout("rsi") self.vm => _,
                inlateout("rdx") self.cpu => _,
                inlateout("r10") exit as *mut Exit as u64 => _,
                inlateout("r8") flags => _,
                out("rcx") _, out("r11") _,
                options(nostack),
            );
        }
        if status == vm::OK { Ok(()) } else { Err(status) }
    }
    fn get_state(&mut self, st: &mut State) -> Result<(), u64> { vm::get_state(self.vm, self.cpu, st) }
    fn set_state(&mut self, st: &State) -> Result<(), u64> { vm::set_state(self.vm, self.cpu, st) }
    fn inject(&mut self, vector: u8, kind: u32, error: Option<u32>) -> Result<(), u64> { vm::inject(self.vm, self.cpu, vector, kind, error) }
}


/// The program around the process: console lines, clock, window.
struct Program<'a> { window: &'a mut Window, line: Vec<u8>, lines: Vec<Vec<u8>>, t0: u64, closed: bool, polls: u32 }

impl Program<'_> {
    fn show(&mut self, status: &[u8]) {
        let mut text = Vec::new();
        text.extend_from_slice(status);
        for l in self.lines.iter().rev().take(20).rev() { text.push(b'\n'); text.extend_from_slice(l); }
        self.window.show(&text);
    }
    fn finish_line(&mut self) {
        let line = core::mem::take(&mut self.line);
        sex_pdx::serial_record!("[crossdress.out] {}", core::str::from_utf8(&line).unwrap_or("?"));
        if self.lines.len() == 64 { self.lines.remove(0); }
        self.lines.push(line);
    }
    fn closing(&mut self) -> bool {
        while let Some(e) = self.window.poll() { if let Event::Close = e { self.closed = true; } }
        self.closed
    }
}

impl Host for Program<'_> {
    fn output(&mut self, _stream: u32, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'\n' => self.finish_line(),
                b'\r' => {}
                0x20..=0x7E if self.line.len() < 200 => self.line.push(b),
                _ => {}
            }
        }
        self.show(b"Crossdress: Windows program running.");
    }
    fn now_ms(&mut self) -> u64 { sex_pdx::get_ticks().saturating_sub(self.t0) }
    fn sleep(&mut self, ms: u32) { let _ = sex_pdx::sys_wait_self_ticks((ms as u64).clamp(1, 64)); }
    fn keep_going(&mut self) -> bool {
        self.polls = self.polls.wrapping_add(1);
        if self.polls % 64 != 0 { return true; }
        !self.closing()
    }
}

fn read_file(path: &[u8], limit: u64) -> Result<Vec<u8>, &'static str> {
    let st = files::direct::stat(path).map_err(|_| "file not found")?;
    if st.dir || st.size == 0 || st.size > limit { return Err("file empty, a folder, or too big"); }
    let mut data = Vec::new();
    data.try_reserve_exact(st.size as usize).map_err(|_| "no memory for the file")?;
    data.resize(st.size as usize, 0);
    match files::direct::read_at(path, 0, &mut data) { Ok(n) if n == data.len() => Ok(data), _ => Err("read failed") }
}

fn lend(machine: u64, ram: &Ram) -> Result<(), (&'static str, u64)> {
    let pages = ram.len() / memory::PAGE;
    for (first, n) in memory::loans(pages, LOAN_PAGES) {
        let mut tries = 0;
        loop {
            match vm::lend(machine, ram.host_address() + first * memory::PAGE, first * memory::PAGE, n as u32, vm::READ | vm::WRITE | vm::EXEC) {
                Ok(()) => break,
                Err(vm::BUSY) if tries < 200 => { tries += 1; let _ = sex_pdx::sys_wait_self_ticks(2); }
                Err(s) => return Err(("lend", s)),
            }
        }
    }
    Ok(())
}

fn stop_line(stop: &Stop) -> (&'static str, u64) {
    match stop {
        Stop::Exited(code) => ("exited", *code as u64),
        Stop::Unhandled { code, .. } => ("unhandled_exception", *code as u64),
        Stop::Unimplemented { .. } => ("unimplemented_import", 0),
        Stop::CellFault { vector, .. } => ("cell_fault", *vector as u64),
        Stop::TripleFault { .. } => ("triple_fault", 0),
        Stop::Unexpected { reason, .. } => ("unexpected_exit", *reason as u64),
        Stop::Hypervisor(s) => ("hypervisor", *s),
        Stop::Host => ("host", 0),
    }
}

fn windows(p: &mut Program, exe: &[u8]) -> Result<Stop, (&'static str, u64)> {
    let ram = Ram::new(CELL_MIB << 20).ok_or(("cell_ram", CELL_MIB as u64))?;
    let mut proc = WinProcess::load(ram, exe).map_err(|_| ("load", 0))?;
    sex_pdx::serial_record!("[crossdress.loaded] base={:#x} entry={:#x} imports={} sections={} cell_mib={}",
        proc.base, proc.entry, proc.imports.len(), proc.image.sections.len(), CELL_MIB);
    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let machine = vm::create().map_err(|s| ("create", s))?;
    let result = (|| {
        lend(machine, &proc.cell.ram)?;
        let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
        vm::set_state(machine, cpu, &proc.initial_state()).map_err(|s| ("set_regs", s))?;
        sex_pdx::serial_record!("[crossdress.run] vm={} pages={}", machine, proc.cell.ram.len() / memory::PAGE);
        let mut vcpu = KernelVcpu::new(machine, cpu);
        Ok(proc.run(&mut vcpu, p))
    })();
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    let stop = result?;
    if !p.line.is_empty() { p.finish_line(); }
    let (why, code) = stop_line(&stop);
    let c = proc.counters;
    sex_pdx::serial_record!("[crossdress.stop] reason={} code={:#x} exits={} calls={} exceptions={} handled={}",
        why, code, c.exits, c.calls, c.exceptions, c.handled);
    if let Stop::Unimplemented { dll, name } = &stop { sex_pdx::serial_record!("[crossdress.unimplemented] {}!{}", dll, name); }
    if let Stop::Unhandled { code, address } = &stop { sex_pdx::serial_record!("[crossdress.unhandled] code={:#x} address={:#x}", code, address); }
    Ok(stop)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Crossdress");
    let mut buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let line = files::args(&mut window, &mut buf).to_vec();
    sex_pdx::serial_record!("[crossdress.start] target={} task={} args_len={}", window.target, window.task, line.len());
    let path: Vec<u8> = line.split(|&c| c == b' ').find(|w| !w.is_empty()).map(|w| w.to_vec()).unwrap_or_default();
    let exe = if path.is_empty() { Ok(sex_crossdress::TEST_EXE.to_vec()) } else { read_file(&path, 256 << 20) };
    let mut p = Program { window: &mut window, line: Vec::new(), lines: Vec::new(), t0: sex_pdx::get_ticks(), closed: false, polls: 0 };
    let text: &[u8] = match exe {
        Err(why) => { sex_pdx::serial_record!("[crossdress.fail] step=read why={}", why); b"Crossdress: could not read that .exe.\nF11 closes." }
        Ok(exe) => match windows(&mut p, &exe) {
            Ok(Stop::Exited(0)) => { sex_pdx::serial_record!("[crossdress.done] target={} task={}", p.window.target, p.window.task); b"Crossdress: the program finished.\nF11 closes." }
            Ok(_) => b"Crossdress: the program stopped (see the log).\nF11 closes.",
            Err(("unavailable", reason)) => {
                sex_pdx::serial_record!("[crossdress.refused] status={} reason={}", vm::UNAVAILABLE, reason);
                b"Crossdress: this machine has no usable VMX.\nF11 closes."
            }
            Err(_) if p.closed => sex_app::launch::exit(0),
            Err((step, status)) => {
                sex_pdx::serial_record!("[crossdress.fail] step={} status={}", step, status);
                b"Crossdress: the cell stopped with an error (see the log).\nF11 closes."
            }
        },
    };
    p.show(text);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[crossdress.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
