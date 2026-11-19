// SPDX-License-Identifier: GPL-3.0-only
//! Drag (D2): a virtual machine, as a confined SexOS program. `run drag` boots the
//! in-tree test guest; `run drag kernel=bzImage [initrd=FILE] [disk=FILE] [ro]
//! [mem=MIB] [-- kernel words]` boots a Linux bzImage from this program's folder.
//!
//! The program holds the Machine capability (program rows only) and nothing else a
//! guest could use: the guest's RAM is this run's own heap, lent to the VM; its disk is
//! a file Files lets this run open; its console is this program's log and window. All
//! device emulation and exit handling is `drag-vmm`; this file is the glue to
//! `sex_pdx::vm` (syscall 110), Files, the clock and the window.
//!
//! Serial log: `[drag.vmm.start]`, `[drag.vmm.loaded]`, then either
//! `[drag.vmm.refused] status=3 reason=...` (no usable VMX: QEMU TCG, AMD before D5) or
//! `[drag.vmm.run]`, one `[drag.serial] ...` per guest console line, `[drag.vmm.stop]`
//! with the reason and counters, and for the test guest `[drag.disk] sector1=...`
//! (what the guest wrote, read back from the file). Failures: `[drag.vmm.fail]`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use drag_vmm::memory::{self, Ram};
use drag_vmm::virtio::{Blk, Disk};
use drag_vmm::vmm::{Host, Machine, Note, Stop, Vcpu};
use sex_app::{files, Event, Window};
use sex_pdx::vm::{self, Exit, State};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[drag.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// Most pages one loan carries (the kernel maps a loan under the program-table lock).
const LOAN_PAGES: u64 = 4096;
/// The test guest's disk file, in this program's folder.
const TEST_DISK: &[u8] = b"drag-test.img";

/// FXSAVE areas: the guest's x87/SSE state is the CPU's while it runs, so the program
/// keeps its own and the guest's apart around every entry.
#[repr(C, align(16))]
struct Fx([u8; 512]);

/// The kernel's vCPU, through syscall 110.
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

/// A disk backed by a file, through Files.
struct FileDisk { path: Vec<u8>, size: u64, read_only: bool }
impl Disk for FileDisk {
    fn len(&self) -> u64 { self.size }
    fn read_only(&self) -> bool { self.read_only }
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Result<(), ()> {
        match files::direct::read_at(&self.path, offset, out) { Ok(n) if n == out.len() => Ok(()), _ => Err(()) }
    }
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), ()> {
        match files::direct::write_at(&self.path, offset, data) { Ok(n) if n == data.len() => Ok(()), _ => Err(()) }
    }
    fn flush(&mut self) -> Result<(), ()> { files::direct::sync().map_err(|_| ()) }
}

fn rdtsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

/// The program around the machine: clock, CPUID, console, window.
struct Program<'a> { window: &'a mut Window, tsc_hz: u64, tsc0: u64, lines: Vec<Vec<u8>>, closed: bool, polls: u32 }

impl Program<'_> {
    fn show(&mut self, status: &[u8]) {
        let mut text = Vec::new();
        text.extend_from_slice(status);
        for l in self.lines.iter().rev().take(12).rev() { text.push(b'\n'); text.extend_from_slice(l); }
        self.window.show(&text);
    }
    fn closing(&mut self) -> bool {
        while let Some(e) = self.window.poll() { if let Event::Close = e { self.closed = true; } }
        self.closed
    }
}

impl Host for Program<'_> {
    fn now_ns(&mut self) -> u64 { ((rdtsc().wrapping_sub(self.tsc0)) as u128 * 1_000_000_000 / self.tsc_hz as u128) as u64 }
    fn tsc(&mut self) -> u64 { rdtsc() }
    fn cpuid(&mut self, leaf: u32, subleaf: u32) -> [u32; 4] {
        let r = core::arch::x86_64::__cpuid_count(leaf, subleaf);
        [r.eax, r.ebx, r.ecx, r.edx]
    }
    fn console(&mut self, line: &[u8]) {
        let text = core::str::from_utf8(line).unwrap_or("?");
        sex_pdx::serial_record!("[drag.serial] {}", text);
        if self.lines.len() == 64 { self.lines.remove(0); }
        self.lines.push(line.to_vec());
        self.show(b"Drag: guest running.");
    }
    fn note(&mut self, note: Note) {
        match note {
            Note::Port { port, write } => sex_pdx::serial_record!("[drag.vmm.note] port={:#x} write={}", port, write as u8),
            Note::Mmio { gpa, write } => sex_pdx::serial_record!("[drag.vmm.note] mmio={:#x} write={}", gpa, write as u8),
            Note::Msr { index, write } => sex_pdx::serial_record!("[drag.vmm.note] msr={:#x} write={} gp=1", index, write as u8),
            Note::Injected { vector } => sex_pdx::serial_record!("[drag.vmm.note] injected={:#x}", vector),
        }
    }
    fn idle(&mut self, until: Option<u64>) -> bool {
        if self.closing() { return false; }
        let ms = match until { Some(t) => (t.saturating_sub(self.now_ns()) / 1_000_000).clamp(1, 64), None => 16 };
        let _ = sex_pdx::sys_wait_self_ticks(ms);
        true
    }
    fn keep_going(&mut self) -> bool {
        self.polls = self.polls.wrapping_add(1);
        if self.polls % 64 != 0 { return true; }
        !self.closing()
    }
}

/// TSC ticks per second, measured against the kernel's millisecond ticks.
fn tsc_hz() -> u64 {
    let t0 = sex_pdx::get_ticks();
    while sex_pdx::get_ticks() == t0 { sex_pdx::sys_yield(); }
    let (start, c0) = (sex_pdx::get_ticks(), rdtsc());
    while sex_pdx::get_ticks() < start + 20 { let _ = sex_pdx::sys_wait_self_ticks(1); }
    let hz = (rdtsc() - c0) * 1000 / (sex_pdx::get_ticks() - start).max(1);
    hz.max(1_000_000)
}

fn read_file(path: &[u8], limit: u64) -> Result<Vec<u8>, &'static str> {
    let st = files::direct::stat(path).map_err(|_| "file not found")?;
    if st.dir || st.size == 0 || st.size > limit { return Err("file empty, a folder, or too big"); }
    let mut data = Vec::new();
    data.try_reserve_exact(st.size as usize).map_err(|_| "no memory for the file")?;
    data.resize(st.size as usize, 0);
    match files::direct::read_at(path, 0, &mut data) { Ok(n) if n == data.len() => Ok(data), _ => Err("read failed") }
}

/// Lends all of `ram` to the VM at guest-physical 0, in loans the kernel can map.
fn lend(machine: u64, ram: &Ram) -> Result<(), (&'static str, u64)> {
    let pages = ram.len() / memory::PAGE;
    for (first, n) in memory::loans(pages, LOAN_PAGES) {
        let mut tries = 0;
        loop {
            match vm::lend(machine, ram.host_address() + first * memory::PAGE, first * memory::PAGE, n as u32, vm::READ | vm::WRITE | vm::EXEC) {
                Ok(()) => break,
                // Some pages are with Denim: the kernel asked for them back.
                Err(vm::BUSY) if tries < 200 => { tries += 1; let _ = sex_pdx::sys_wait_self_ticks(2); }
                Err(s) => return Err(("lend", s)),
            }
        }
    }
    Ok(())
}

fn stop_line(stop: &Stop) -> (&'static str, u64, u64) {
    match *stop {
        Stop::Halted { rip } => ("halted", rip, 0),
        Stop::TripleFault { rip } => ("triple_fault", rip, 0),
        Stop::EntryFailed { reason } => ("entry_failed", 0, reason as u64),
        Stop::Hypervisor { status } => ("hypervisor", 0, status),
        Stop::Mmio { gpa, rip, .. } => ("mmio_undecoded", rip, gpa),
        Stop::LostMemory { gpa } => ("lost_memory", 0, gpa),
        Stop::Unsupported { reason, rip, .. } => ("unsupported_exit", rip, reason as u64),
        Stop::Host => ("host", 0, 0),
    }
}

fn guest(p: &mut Program, args: &drag_vmm::Args) -> Result<(), (&'static str, u64)> {
    let test = args.kernel.is_none();
    // The disk: the named file, or for the test guest a fresh image in our folder.
    let disk = match (&args.disk, test) {
        (Some(path), _) => {
            let st = files::direct::stat(path).map_err(|e| ("disk_stat", e as u64))?;
            Some(FileDisk { path: path.clone(), size: st.size & !511, read_only: args.read_only })
        }
        (None, true) => {
            files::write(p.window, TEST_DISK, &drag_vmm::test_disk()).map_err(|e| ("disk_create", e as u64))?;
            Some(FileDisk { path: TEST_DISK.to_vec(), size: drag_vmm::test_disk().len() as u64, read_only: false })
        }
        (None, false) => None,
    };
    let kernel = match &args.kernel {
        Some(path) => read_file(path, 64 << 20).map_err(|_| ("kernel_read", 0))?,
        None => drag_vmm::TEST_GUEST.to_vec(),
    };
    let initrd = match &args.initrd { Some(path) => Some(read_file(path, 512 << 20).map_err(|_| ("initrd_read", 0))?), None => None };
    let ram = Ram::new((args.mem_mib as usize) << 20).ok_or(("guest_ram", args.mem_mib as u64))?;
    let blk = disk.map(|d| Blk::new(d, drag_vmm::BLK_BASE, drag_vmm::BLK_IRQ));
    let mut m = Machine::new(ram, blk);
    let mut cmdline = Vec::new();
    cmdline.extend_from_slice(drag_vmm::DEFAULT_CMDLINE);
    if !args.cmdline.is_empty() { cmdline.push(b' '); cmdline.extend_from_slice(&args.cmdline); }
    let boot = m.load_linux(&kernel, initrd.as_deref(), &cmdline).map_err(|_| ("load", 0))?;
    drop(kernel);
    sex_pdx::serial_record!("[drag.vmm.loaded] guest={} protocol={:#x} entry={} rip={:#x} kernel={:#x}+{:#x} mem_mib={} disk={}",
        if test { "test" } else { "bzimage" }, boot.header.version, if boot.header.entry64 { 64 } else { 32 }, boot.entry.regs.rip,
        boot.kernel.0, boot.kernel.1, args.mem_mib, m.blk.as_ref().map_or(0, |b| b.disk.len() / 512));

    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let machine = vm::create().map_err(|s| ("create", s))?;
    let result = (|| {
        lend(machine, &m.ram)?;
        let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
        vm::set_state(machine, cpu, &boot.entry).map_err(|s| ("set_regs", s))?;
        sex_pdx::serial_record!("[drag.vmm.run] vm={} pages={}", machine, m.ram.len() / memory::PAGE);
        p.show(b"Drag: guest running.");
        let mut vcpu = KernelVcpu::new(machine, cpu);
        let stop = m.run(&mut vcpu, p, u64::MAX);
        if !m.console_pending().is_empty() { p.console(&m.console_pending().to_vec()); }
        let (why, rip, detail) = stop_line(&stop);
        let c = m.counters;
        sex_pdx::serial_record!("[drag.vmm.stop] reason={} rip={:#x} detail={:#x} exits={} io={} mmio={} cpuid={} msr={} hlt={} injected={}",
            why, rip, detail, c.exits, c.io, c.mmio, c.cpuid, c.msr, c.hlt, c.injected);
        if let Stop::Mmio { bytes, len, .. } = stop { sex_pdx::serial_record!("[drag.vmm.mmio] bytes={:02x?}", &bytes[..len as usize]); }
        if let Some(b) = &m.blk {
            let s = b.stats;
            sex_pdx::serial_record!("[drag.vmm.blk] requests={} read={} written={} errors={}", s.requests, s.read_bytes, s.written_bytes, s.errors);
        }
        Ok(stop)
    })();
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    let stop = result?;
    if test {
        let mut sector = [0u8; 31];
        let _ = files::direct::read_at(TEST_DISK, 512, &mut sector);
        sex_pdx::serial_record!("[drag.disk] sector1={}", core::str::from_utf8(&sector).unwrap_or("?"));
    }
    match stop { Stop::Halted { .. } => Ok(()), other => Err((stop_line(&other).0, 0)) }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Drag");
    let mut buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let line = files::args(&mut window, &mut buf).to_vec();
    sex_pdx::serial_record!("[drag.vmm.start] target={} task={} args_len={}", window.target, window.task, line.len());
    let text: &[u8] = match drag_vmm::Args::parse(&line) {
        Err(why) => {
            sex_pdx::serial_record!("[drag.vmm.fail] step=args why={}", why);
            b"Drag: run drag [kernel=FILE] [initrd=FILE] [disk=FILE] [ro] [mem=MIB] [-- words]\nF11 closes."
        }
        Ok(args) => {
            let mut p = Program { window: &mut window, tsc_hz: 0, tsc0: rdtsc(), lines: Vec::new(), closed: false, polls: 0 };
            p.tsc_hz = tsc_hz();
            let r = guest(&mut p, &args);
            let closed = p.closed;
            match r {
                Ok(()) => { sex_pdx::serial_record!("[drag.vmm.done] target={} task={}", window.target, window.task); b"Drag: the guest finished.\nF11 closes." }
                Err(("unavailable", reason)) => {
                    sex_pdx::serial_record!("[drag.vmm.refused] status={} reason={}", vm::UNAVAILABLE, reason);
                    b"Drag: this machine has no usable VMX.\nF11 closes."
                }
                Err(_) if closed => window.close(),
                Err((step, status)) => {
                    sex_pdx::serial_record!("[drag.vmm.fail] step={} status={}", step, status);
                    b"Drag: the guest stopped with an error (see the log).\nF11 closes."
                }
            }
        }
    };
    window.show(text);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[drag.vmm.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
