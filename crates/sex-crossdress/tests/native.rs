// SPDX-License-Identifier: GPL-3.0-only
//! The native test vCPU: runs a cell's user code directly on the Linux test host, with no
//! hypervisor, so the Windows side can be tested where /dev/kvm is missing (this is the
//! host-test counterpart of CW2's in-SAS mode).
//!
//! The cell's user pages are mapped at their own guest addresses in the test process
//! (they are ordinary user addresses on Linux too) and copied in before each entry and
//! back after each exit, so the host side (`WinProcess`, `Host`) sees the same memory it
//! would under SYS_VM or KVM. Entry and exit both go through signal handlers: SIGUSR1
//! swaps the host's registers for the guest's, and the guest's `out 0xE0, eax` (#GP in
//! Linux ring 3, so SIGSEGV) swaps them back and reports the same I/O exit SYS_VM gives.
//! GS base is the TEB, as in the cell. Guest exceptions are not dispatched here: a real
//! fault is reported as an unexpected exit (the exception paths are gated on KVM).
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#![allow(dead_code)]

use drag_vmm::vmm::Vcpu;
use sex_crossdress::cell::{Access, Cell, PAGE};
use sex_crossdress::process::TEB_VA;
use sex_pdx::vm::{exit, Exit, Regs, State};
use std::sync::Mutex;

mod common;

/// One native test at a time: the guest state and the host's saved context are global,
/// and only one guest may own these addresses.
pub static NATIVE: Mutex<()> = Mutex::new(());

static mut HOST_GREGS: [i64; 23] = [0; 23];
static mut HOST_FP: [u8; 512] = [0; 512];
static mut GUEST: Regs = Regs { rax: 0, rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0, rbp: 0, rsp: 0, r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0, rip: 0, rflags: 0 };
static mut GUEST_FP: [u8; 512] = [0; 512];
static mut FAULT: (i32, u64) = (0, 0);

/// libc's gregs order: R8..R15, RDI, RSI, RBP, RBX, RDX, RAX, RCX, RSP, RIP, EFL, ...
fn to_gregs(r: &Regs, g: &mut [i64; 23]) {
    let v = [r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15, r.rdi, r.rsi, r.rbp, r.rbx, r.rdx, r.rax, r.rcx, r.rsp, r.rip];
    for (i, x) in v.iter().enumerate() { g[i] = *x as i64; }
    g[libc::REG_EFL as usize] = ((r.rflags & 0x0CD5) | 0x202) as i64;
}
fn from_gregs(g: &[i64; 23]) -> Regs {
    let q = |i: libc::c_int| g[i as usize] as u64;
    Regs { r8: q(libc::REG_R8), r9: q(libc::REG_R9), r10: q(libc::REG_R10), r11: q(libc::REG_R11), r12: q(libc::REG_R12), r13: q(libc::REG_R13),
        r14: q(libc::REG_R14), r15: q(libc::REG_R15), rdi: q(libc::REG_RDI), rsi: q(libc::REG_RSI), rbp: q(libc::REG_RBP), rbx: q(libc::REG_RBX),
        rdx: q(libc::REG_RDX), rax: q(libc::REG_RAX), rcx: q(libc::REG_RCX), rsp: q(libc::REG_RSP), rip: q(libc::REG_RIP), rflags: q(libc::REG_EFL) }
}

unsafe fn gregs(uc: *mut libc::c_void) -> &'static mut [i64; 23] { &mut (*(uc as *mut libc::ucontext_t)).uc_mcontext.gregs }
unsafe fn fpregs(uc: *mut libc::c_void) -> *mut u8 { (*(uc as *mut libc::ucontext_t)).uc_mcontext.fpregs as *mut u8 }

/// SIGUSR1: enter the guest. The host's context is kept and the guest's loaded.
extern "C" fn enter(_: libc::c_int, _: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    unsafe {
        let g = gregs(uc);
        HOST_GREGS = *g;
        let fp = fpregs(uc);
        std::ptr::copy_nonoverlapping(fp, std::ptr::addr_of_mut!(HOST_FP) as *mut u8, 512);
        std::ptr::copy_nonoverlapping(std::ptr::addr_of!(GUEST_FP) as *const u8, fp, 512);
        let guest = GUEST;
        to_gregs(&guest, g);
    }
}

/// SIGSEGV/SIGBUS/SIGILL/SIGFPE/SIGTRAP from the guest: back to the host.
extern "C" fn leave(sig: libc::c_int, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    unsafe {
        let g = gregs(uc);
        GUEST = from_gregs(g);
        FAULT = (sig, (*info).si_addr() as u64);
        let fp = fpregs(uc);
        std::ptr::copy_nonoverlapping(fp, std::ptr::addr_of_mut!(GUEST_FP) as *mut u8, 512);
        std::ptr::copy_nonoverlapping(std::ptr::addr_of!(HOST_FP) as *const u8, fp, 512);
        *g = HOST_GREGS;
    }
}

fn install() {
    unsafe {
        // The exit handler runs on its own stack, never on the guest's.
        let size = 256 << 10;
        let stack = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0);
        assert_ne!(stack, libc::MAP_FAILED);
        let ss = libc::stack_t { ss_sp: stack, ss_flags: 0, ss_size: size };
        assert_eq!(libc::sigaltstack(&ss, std::ptr::null_mut()), 0);
        for (sig, f) in [(libc::SIGUSR1, enter as *const () as usize), (libc::SIGSEGV, leave as *const () as usize), (libc::SIGBUS, leave as *const () as usize),
            (libc::SIGILL, leave as *const () as usize), (libc::SIGFPE, leave as *const () as usize), (libc::SIGTRAP, leave as *const () as usize)] {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = f;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_NODEFER;
            assert_eq!(libc::sigaction(sig, &sa, std::ptr::null_mut()), 0);
        }
        let mut fp = [0u8; 512];
        fp[0..2].copy_from_slice(&0x037Fu16.to_le_bytes());
        fp[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
        fp[28..32].copy_from_slice(&0xFFFFu32.to_le_bytes()); // MXCSR mask
        GUEST_FP = fp;
    }
}

fn restore_default() {
    unsafe {
        for sig in [libc::SIGUSR1, libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGTRAP] { libc::signal(sig, libc::SIG_DFL); }
        let ss = libc::stack_t { ss_sp: std::ptr::null_mut(), ss_flags: libc::SS_DISABLE, ss_size: 0 };
        libc::sigaltstack(&ss, std::ptr::null_mut());
    }
}

fn prot(a: Access) -> i32 {
    libc::PROT_READ | if a.write { libc::PROT_WRITE } else { 0 } | if a.execute { libc::PROT_EXEC } else { 0 }
}

/// A vCPU that runs the cell's user code natively. Holds the address range it mapped.
pub struct Native { cell: *mut Cell, mapped: Vec<(u64, u64)>, state: State, pending: bool, pub exits: u64 }

impl Native {
    /// Takes over the guest addresses for `cell` (None when Linux already uses one).
    pub fn new(cell: &mut Cell) -> Option<Native> {
        install();
        unsafe {
            // GS base = the TEB, as the cell's state sets it.
            if libc::syscall(libc::SYS_arch_prctl, 0x1001 /* ARCH_SET_GS */, TEB_VA) != 0 { return None; }
        }
        let mut n = Native { cell, mapped: Vec::new(), state: State::default(), pending: false, exits: 0 };
        if !n.map_new() { return None; }
        Some(n)
    }

    fn cell(&self) -> &mut Cell { unsafe { &mut *self.cell } }

    /// Maps the cell's user pages that are not mapped here yet.
    fn map_new(&mut self) -> bool {
        let maps: Vec<_> = self.cell().maps().iter().filter(|m| m.access.user).copied().collect();
        for m in maps {
            let bytes = m.pages * PAGE;
            // A run that NtProtectVirtualMemory split off lies inside one already mapped.
            if self.mapped.iter().any(|&(va, n)| va <= m.va && m.va + bytes <= va + n) { continue; }
            let p = unsafe { libc::mmap(m.va as *mut _, bytes as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE, -1, 0) };
            if p == libc::MAP_FAILED || p as u64 != m.va { eprintln!("[crossdress.native] cannot map {:#x}+{:#x}", m.va, bytes); return false; }
            self.mapped.push((m.va, bytes));
        }
        true
    }

    /// Cell memory to the native pages (then each page gets its cell rights).
    fn copy_in(&mut self) {
        let ram = self.cell().ram.host_address();
        for m in self.cell().maps().iter().filter(|m| m.access.user) {
            let bytes = (m.pages * PAGE) as usize;
            unsafe {
                libc::mprotect(m.va as *mut _, bytes, libc::PROT_READ | libc::PROT_WRITE);
                std::ptr::copy_nonoverlapping((ram + m.gpa) as *const u8, m.va as *mut u8, bytes);
                libc::mprotect(m.va as *mut _, bytes, prot(m.access));
            }
        }
    }

    /// Native pages back to cell memory (only writable ones can have changed).
    fn copy_out(&mut self) {
        let ram = self.cell().ram.host_address();
        for m in self.cell().maps().iter().filter(|m| m.access.user && m.access.write) {
            unsafe { std::ptr::copy_nonoverlapping(m.va as *const u8, (ram + m.gpa) as *mut u8, (m.pages * PAGE) as *const () as usize); }
        }
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        for &(va, bytes) in &self.mapped { unsafe { libc::munmap(va as *mut _, bytes as *const () as usize); } }
        restore_default();
    }
}

impl Vcpu for Native {
    fn run(&mut self, e: &mut Exit, load_regs: bool) -> Result<(), u64> {
        let regs = if load_regs { e.regs } else if self.pending { self.state.regs } else { unsafe { GUEST } };
        self.pending = false;
        if !self.map_new() { return Err(2); }
        self.copy_in();
        unsafe {
            GUEST = regs;
            FAULT = (0, 0);
            libc::raise(libc::SIGUSR1);
        }
        self.copy_out();
        self.exits += 1;
        let (regs, (sig, addr)) = unsafe { (GUEST, FAULT) };
        *e = Exit { regs, ..Exit::default() };
        let mut op = [0u8; 2];
        let is_out = sig == libc::SIGSEGV && self.cell().read_user(regs.rip, &mut op).is_some() && op == [0xE7, 0xE0];
        if is_out {
            e.reason = exit::IO;
            e.port = 0xE0;
            e.io_size = 4;
            e.io_flags = 0;
            e.instruction_len = 2;
        } else {
            eprintln!("[crossdress.native] guest fault sig={sig} addr={addr:#x} rip={:#x} rsp={:#x}", regs.rip, regs.rsp);
            e.reason = 0xFFFF;
        }
        Ok(())
    }
    fn get_state(&mut self, st: &mut State) -> Result<(), u64> { *st = self.state; st.regs = unsafe { GUEST }; Ok(()) }
    fn set_state(&mut self, st: &State) -> Result<(), u64> { self.state = *st; self.pending = true; Ok(()) }
    fn inject(&mut self, _: u8, _: u32, _: Option<u32>) -> Result<(), u64> { Err(1) }
}

#[derive(Default)]
struct Console { text: Vec<u8> }
impl sex_crossdress::process::Host for Console {
    fn output(&mut self, _: u32, b: &[u8]) { self.text.extend_from_slice(b); }
    fn now_ms(&mut self) -> u64 { 1 }
    fn sleep(&mut self, _: u32) {}
}

#[test]
fn the_windows_clap_plugin_runs_natively() {
    use sex_crossdress::clap::{ClapPlugin, ParamChange};
    let _one = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let mut plugin = ClapPlugin::load(drag_vmm::memory::Ram::new(64 << 20).unwrap(), sex_crossdress::TEST_CLAP).unwrap();
    let Some(mut cpu) = Native::new(&mut plugin.process.cell) else { println!("[crossdress.native] SKIP guest addresses taken"); return; };
    let mut con = Console::default();
    plugin.open(&mut cpu, &mut con, "C:\\gain.clap").unwrap();
    assert_eq!(plugin.descriptor.name, "Crossdress Test Gain");
    assert_eq!((plugin.requests.callback, plugin.main_thread_calls), (1, 1));
    plugin.activate(&mut cpu, &mut con, 48_000.0, 512).unwrap();
    let l: Vec<f32> = (0..256).map(|i| i as f32 / 256.0).collect();
    let r: Vec<f32> = l.iter().map(|x| -x).collect();
    let (mut ol, mut or) = (vec![0f32; 256], vec![0f32; 256]);
    let pushed = plugin.process(&mut cpu, &mut con, [&l, &r], [&mut ol, &mut or], &[ParamChange { time: 10, id: 0, value: 0.25 }]).unwrap();
    for i in 0..256 { let g = if i < 10 { 1.0 } else { 0.25f32 }; assert_eq!((ol[i], or[i]), (l[i] * g, r[i] * g), "frame {i}"); }
    assert_eq!(pushed, [ParamChange { time: 10, id: 0, value: 0.25 }]);
    plugin.close(&mut cpu, &mut con).unwrap();
    println!("[crossdress.native] ok: the CLAP plugin ran natively, exits={}", cpu.exits);
}

#[test]
fn wines_ntdll_runs_natively() {
    let _one = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(dll) = common::wine_dll("ntdll.dll") else { println!("[crossdress.native.wine] SKIP no CROSSDRESS_WINE_DIR (scripts/fetch_wine_pe.sh)"); return; };
    let mut p = sex_crossdress::process::WinProcess::load_dll(drag_vmm::memory::Ram::new(128 << 20).unwrap(), &dll).unwrap();
    sex_crossdress::nt::prepare_ntdll(&mut p).unwrap();
    println!("[crossdress.native.wine] ntdll base={:#x} exports={} syscalls={}", p.base, p.exports.len(), p.syscall_names.len());
    let Some(mut cpu) = Native::new(&mut p.cell) else { println!("[crossdress.native.wine] SKIP guest addresses taken"); return; };
    for line in common::exercise_ntdll(&mut cpu, &mut p) { println!("[crossdress.native.wine] {line}"); }
}

#[test]
fn a_program_runs_on_wines_kernel32_natively() {
    let _one = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut p) = common::wine_program() else { println!("[crossdress.native.wine] SKIP no CROSSDRESS_WINE_DIR"); return; };
    let Some(mut cpu) = Native::new(&mut p.cell) else { println!("[crossdress.native.wine] SKIP guest addresses taken"); return; };
    let mut out = common::Keep::default();
    let stop = sex_crossdress::wine::run(&mut p, &mut cpu, &mut out);
    let text = String::from_utf8_lossy(&out.text);
    for line in text.lines() { println!("[crossdress.native.wine.out] {line}"); }
    println!("[crossdress.native.wine] stop={stop:?} counters={:?}", p.counters);
    assert!(text.contains(sex_crossdress::WINE_PASS), "the program's checks passed on Wine's kernel32");
    assert_eq!(stop, sex_crossdress::process::Stop::Exited(0));
}

#[test]
fn a_crt_program_runs_on_wines_ucrtbase_natively() {
    let _one = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut p) = common::crt_program() else { println!("[crossdress.native.crt] SKIP no CROSSDRESS_WINE_DIR"); return; };
    let Some(mut cpu) = Native::new(&mut p.cell) else { println!("[crossdress.native.crt] SKIP guest addresses taken"); return; };
    let mut out = common::Keep::default();
    let stop = sex_crossdress::wine::run(&mut p, &mut cpu, &mut out);
    let text = String::from_utf8_lossy(&out.text);
    for line in text.lines() { println!("[crossdress.native.crt.out] {line}"); }
    println!("[crossdress.native.crt] stop={stop:?} counters={:?}", p.counters);
    assert!(text.contains(sex_crossdress::CRT_PASS), "the program's checks passed on Wine's ucrtbase");
    assert_eq!(stop, sex_crossdress::process::Stop::Exited(0));
}

#[test]
fn an_msvc_style_program_runs_through_api_sets_natively() {
    let _one = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut p) = common::msvc_program() else { println!("[crossdress.native.msvc] SKIP no CROSSDRESS_WINE_DIR"); return; };
    let Some(mut cpu) = Native::new(&mut p.cell) else { println!("[crossdress.native.msvc] SKIP guest addresses taken"); return; };
    let mut out = common::Keep::default();
    let stop = sex_crossdress::wine::run(&mut p, &mut cpu, &mut out);
    let text = String::from_utf8_lossy(&out.text);
    for line in text.lines() { println!("[crossdress.native.msvc.out] {line}"); }
    println!("[crossdress.native.msvc] stop={stop:?} counters={:?}", p.counters);
    assert!(text.contains(sex_crossdress::MSVC_PASS), "the program's checks passed through API sets and vcruntime140");
    assert_eq!(stop, sex_crossdress::process::Stop::Exited(0));
}
