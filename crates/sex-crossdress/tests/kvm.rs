// SPDX-License-Identifier: GPL-3.0-only
//! Runs the CW1 test program in a real VT-x/SVM cell on Linux KVM, through the same
//! `WinProcess` the SexOS program uses. KVM stands in for SexOS's SYS_VM: both give the
//! host one exit at a time with the guest's registers, and both run guest code natively.
//! Skipped (with a SKIP line) where /dev/kvm is missing or not usable, as in QEMU TCG
//! containers; CI makes /dev/kvm usable where the runner has it.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use drag_vmm::memory::Ram;
use drag_vmm::vmm::Vcpu;
use sex_crossdress::process::{Host, WinProcess, Stop};
use sex_pdx::vm::{exit, Exit, Regs, Segment, State, IO_IN};
use std::os::fd::RawFd;

const KVMIO: u64 = 0xAE;
const fn io(nr: u64) -> u64 { (KVMIO << 8) | nr }
const fn iow(nr: u64, size: u64) -> u64 { (1 << 30) | (size << 16) | (KVMIO << 8) | nr }
const fn ior(nr: u64, size: u64) -> u64 { (2 << 30) | (size << 16) | (KVMIO << 8) | nr }
const fn iowr(nr: u64, size: u64) -> u64 { (3 << 30) | (size << 16) | (KVMIO << 8) | nr }

const KVM_GET_API_VERSION: u64 = io(0x00);
const KVM_CREATE_VM: u64 = io(0x01);
const KVM_GET_VCPU_MMAP_SIZE: u64 = io(0x04);
const KVM_GET_SUPPORTED_CPUID: u64 = iowr(0x05, 8);
const KVM_CREATE_VCPU: u64 = io(0x41);
const KVM_SET_USER_MEMORY_REGION: u64 = iow(0x46, 32);
const KVM_SET_TSS_ADDR: u64 = io(0x47);
const KVM_RUN: u64 = io(0x80);
const KVM_GET_REGS: u64 = ior(0x81, 144);
const KVM_SET_REGS: u64 = iow(0x82, 144);
const KVM_GET_SREGS: u64 = ior(0x83, 312);
const KVM_SET_SREGS: u64 = iow(0x84, 312);
const KVM_SET_CPUID2: u64 = iow(0x90, 8);

const KVM_EXIT_IO: u32 = 2;
const KVM_EXIT_HLT: u32 = 5;
const KVM_EXIT_SHUTDOWN: u32 = 8;
const KVM_EXIT_FAIL_ENTRY: u32 = 9;
const KVM_EXIT_INTR: u32 = 10;

fn ioctl(fd: RawFd, req: u64, arg: u64) -> i64 {
    unsafe { libc::ioctl(fd, req as _, arg) as i64 }
}

/// kvm_regs order: rax rbx rcx rdx rsi rdi rsp rbp r8..r15 rip rflags.
fn regs_to_kvm(r: &Regs) -> [u64; 18] {
    [r.rax, r.rbx, r.rcx, r.rdx, r.rsi, r.rdi, r.rsp, r.rbp, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15, r.rip, r.rflags]
}
fn regs_from_kvm(k: &[u64; 18]) -> Regs {
    Regs { rax: k[0], rbx: k[1], rcx: k[2], rdx: k[3], rsi: k[4], rdi: k[5], rsp: k[6], rbp: k[7], r8: k[8], r9: k[9], r10: k[10], r11: k[11],
        r12: k[12], r13: k[13], r14: k[14], r15: k[15], rip: k[16], rflags: k[17] }
}

/// Where each of State's segments (ES CS SS DS FS GS LDTR TR) sits in kvm_sregs.
const SEG_AT: [usize; 8] = [48, 0, 120, 24, 72, 96, 168, 144];

fn seg_to_kvm(s: &Segment, out: &mut [u8]) {
    let a = s.access;
    out[0..8].copy_from_slice(&s.base.to_le_bytes());
    out[8..12].copy_from_slice(&s.limit.to_le_bytes());
    out[12..14].copy_from_slice(&s.selector.to_le_bytes());
    let bits = [a & 0xF, (a >> 7) & 1, (a >> 5) & 3, (a >> 14) & 1, (a >> 4) & 1, (a >> 13) & 1, (a >> 15) & 1, (a >> 12) & 1, (a >> 16) & 1];
    for (i, b) in bits.iter().enumerate() { out[14 + i] = *b as u8; }
    out[23] = 0;
}
fn seg_from_kvm(k: &[u8]) -> Segment {
    let a = k[14] as u32 | (k[18] as u32) << 4 | (k[16] as u32) << 5 | (k[15] as u32) << 7 | (k[21] as u32) << 12
        | (k[19] as u32) << 13 | (k[17] as u32) << 14 | (k[20] as u32) << 15 | (k[22] as u32) << 16;
    Segment {
        base: u64::from_le_bytes(k[0..8].try_into().unwrap()),
        limit: u32::from_le_bytes(k[8..12].try_into().unwrap()),
        access: a,
        selector: u16::from_le_bytes(k[12..14].try_into().unwrap()),
        _pad: [0; 3],
    }
}

struct Kvm { vcpu: RawFd, run: *mut u8, ram: *const Ram }

impl Kvm {
    fn sregs(&self) -> [u8; 312] {
        let mut s = [0u8; 312];
        assert_eq!(ioctl(self.vcpu, KVM_GET_SREGS, s.as_mut_ptr() as u64), 0, "KVM_GET_SREGS");
        s
    }
    fn regs(&self) -> Regs {
        let mut k = [0u64; 18];
        assert_eq!(ioctl(self.vcpu, KVM_GET_REGS, k.as_mut_ptr() as u64), 0, "KVM_GET_REGS");
        regs_from_kvm(&k)
    }
    fn set_regs(&self, r: &Regs) -> Result<(), u64> {
        let k = regs_to_kvm(r);
        if ioctl(self.vcpu, KVM_SET_REGS, k.as_ptr() as u64) == 0 { Ok(()) } else { Err(1) }
    }
    fn u64_at(s: &[u8], at: usize) -> u64 { u64::from_le_bytes(s[at..at + 8].try_into().unwrap()) }
}

impl Vcpu for Kvm {
    fn run(&mut self, e: &mut Exit, load_regs: bool) -> Result<(), u64> {
        if load_regs { self.set_regs(&e.regs)?; }
        if ioctl(self.vcpu, KVM_RUN, 0) != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) { *e = Exit { reason: exit::EXTERNAL_INTERRUPT, regs: self.regs(), ..Exit::default() }; return Ok(()); }
            panic!("KVM_RUN: {err}");
        }
        let reason = unsafe { (self.run.add(8) as *const u32).read_unaligned() };
        *e = Exit { regs: self.regs(), ..Exit::default() };
        match reason {
            KVM_EXIT_IO => {
                let (dir, size, port, count) = unsafe { (*self.run.add(32), *self.run.add(33), (self.run.add(34) as *const u16).read_unaligned(), (self.run.add(36) as *const u32).read_unaligned()) };
                e.reason = exit::IO;
                e.port = port;
                e.io_size = size;
                e.io_flags = if dir == 0 { IO_IN } else { 0 } | if count > 1 { 2 } else { 0 };
                // SYS_VM reports RIP at the `out`; KVM may already be past it. Our only
                // `out` is the 2-byte `out 0xE0, eax`: find it.
                let st = self.state_now();
                let at = |rip: u64| { let mut b = [0u8; 2]; drag_vmm::x86::read_linear(unsafe { &*self.ram }, &st, rip, &mut b).map(|_| b) };
                if at(e.regs.rip) != Some([0xE7, 0xE0]) && at(e.regs.rip.wrapping_sub(2)) == Some([0xE7, 0xE0]) { e.regs.rip -= 2; }
                e.instruction_len = 2;
            }
            KVM_EXIT_HLT => e.reason = exit::HLT,
            KVM_EXIT_SHUTDOWN => e.reason = exit::TRIPLE_FAULT,
            KVM_EXIT_FAIL_ENTRY => e.reason = exit::ENTRY_FAILURE | 33,
            KVM_EXIT_INTR => e.reason = exit::EXTERNAL_INTERRUPT,
            r => e.reason = 1000 + r,
        }
        Ok(())
    }
    fn get_state(&mut self, st: &mut State) -> Result<(), u64> { *st = self.state_now(); st.regs = self.regs(); Ok(()) }
    fn set_state(&mut self, st: &State) -> Result<(), u64> {
        let mut s = self.sregs();
        for (i, at) in SEG_AT.iter().enumerate() { seg_to_kvm(&st.segments[i], &mut s[*at..*at + 24]); }
        s[192..200].copy_from_slice(&st.gdtr_base.to_le_bytes());
        s[200..202].copy_from_slice(&(st.gdtr_limit as u16).to_le_bytes());
        s[208..216].copy_from_slice(&st.idtr_base.to_le_bytes());
        s[216..218].copy_from_slice(&(st.idtr_limit as u16).to_le_bytes());
        for (at, v) in [(224, st.cr0), (240, st.guest_cr3), (248, st.cr4), (264, st.efer)] { s[at..at + 8].copy_from_slice(&v.to_le_bytes()); }
        if ioctl(self.vcpu, KVM_SET_SREGS, s.as_ptr() as u64) != 0 { panic!("KVM_SET_SREGS: {}", std::io::Error::last_os_error()); }
        self.set_regs(&st.regs)
    }
    fn inject(&mut self, _: u8, _: u32, _: Option<u32>) -> Result<(), u64> { Err(1) }
}

impl Kvm {
    fn state_now(&self) -> State {
        let s = self.sregs();
        let mut st = State::default();
        for (i, at) in SEG_AT.iter().enumerate() { st.segments[i] = seg_from_kvm(&s[*at..*at + 24]); }
        st.gdtr_base = Self::u64_at(&s, 192);
        st.idtr_base = Self::u64_at(&s, 208);
        st.gdtr_limit = u16::from_le_bytes([s[200], s[201]]) as u32;
        st.idtr_limit = u16::from_le_bytes([s[216], s[217]]) as u32;
        st.cr0 = Self::u64_at(&s, 224);
        st.guest_cr3 = Self::u64_at(&s, 240);
        st.cr4 = Self::u64_at(&s, 248);
        st.efer = Self::u64_at(&s, 264);
        st
    }
}

#[derive(Default)]
struct Console { text: Vec<u8> }
impl Host for Console {
    fn output(&mut self, _: u32, b: &[u8]) { self.text.extend_from_slice(b); }
    fn now_ms(&mut self) -> u64 { 1 }
    fn sleep(&mut self, _: u32) {}
}

fn open_kvm() -> Option<RawFd> {
    let fd = unsafe { libc::open(c"/dev/kvm".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 { return None; }
    if ioctl(fd, KVM_GET_API_VERSION, 0) != 12 { return None; }
    Some(fd)
}

#[test]
fn the_test_program_runs_in_a_kvm_cell() {
    let Some(kvm) = open_kvm() else {
        println!("[crossdress.kvm] SKIP no usable /dev/kvm: the cell did not run on hardware here");
        return;
    };
    // CROSSDRESS_EXE runs another build of the test program (the gate's rebuild with the
    // runner's own LLVM) instead of the checked-in one.
    let exe = match std::env::var_os("CROSSDRESS_EXE") {
        Some(path) => { println!("[crossdress.kvm] exe={}", path.to_string_lossy()); std::fs::read(path).expect("CROSSDRESS_EXE") }
        None => sex_crossdress::TEST_EXE.to_vec(),
    };
    let mut proc = WinProcess::load(Ram::new(64 << 20).unwrap(), &exe).unwrap();
    let vm = ioctl(kvm, KVM_CREATE_VM, 0) as RawFd;
    assert!(vm >= 0, "KVM_CREATE_VM");
    let _ = ioctl(vm, KVM_SET_TSS_ADDR, 0xFFFB_D000);
    let region: [u64; 4] = [0, 0, proc.cell.ram.len(), proc.cell.ram.host_address()];
    assert_eq!(ioctl(vm, KVM_SET_USER_MEMORY_REGION, region.as_ptr() as u64), 0, "KVM_SET_USER_MEMORY_REGION");
    let vcpu = ioctl(vm, KVM_CREATE_VCPU, 0) as RawFd;
    assert!(vcpu >= 0, "KVM_CREATE_VCPU");
    // The guest's CPUID must offer NX (EFER.NXE) and the SSE state the program uses.
    let mut cpuid = vec![0u8; 8 + 256 * 40];
    cpuid[0..4].copy_from_slice(&256u32.to_le_bytes());
    assert_eq!(ioctl(kvm, KVM_GET_SUPPORTED_CPUID, cpuid.as_mut_ptr() as u64), 0, "KVM_GET_SUPPORTED_CPUID");
    assert_eq!(ioctl(vcpu, KVM_SET_CPUID2, cpuid.as_ptr() as u64), 0, "KVM_SET_CPUID2");
    let size = ioctl(kvm, KVM_GET_VCPU_MMAP_SIZE, 0) as usize;
    let run = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, vcpu, 0) };
    assert_ne!(run, libc::MAP_FAILED);
    let mut cpu = Kvm { vcpu, run: run as *mut u8, ram: &proc.cell.ram };
    cpu.set_state(&proc.initial_state()).unwrap();
    let mut console = Console::default();
    let stop = proc.run(&mut cpu, &mut console);
    let text = String::from_utf8_lossy(&console.text);
    for line in text.lines() { println!("[crossdress.kvm.out] {line}"); }
    println!("[crossdress.kvm] stop={stop:?} counters={:?} last_fault={:?}", proc.counters, proc.last_fault);
    assert_eq!(stop, Stop::Exited(0), "the program finished cleanly");
    assert!(text.contains(sex_crossdress::TEST_PASS), "every check in the program passed");
    assert_eq!(proc.counters.handled, 5, "five hardware exceptions went through the vectored handler");
}
