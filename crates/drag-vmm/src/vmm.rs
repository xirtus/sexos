// SPDX-License-Identifier: GPL-3.0-only
//! The machine and its exit loop: one vCPU, guest RAM at 0, the PC legacy devices
//! (16550 COM1, 8259 PIC pair, 8254 PIT, a CMOS stub, port 0x61, 0xE9 debug port) and
//! virtio-blk over virtio-mmio. `Machine::run` enters the guest through a [`Vcpu`]
//! (the kernel's `SYS_VM` in the program; a scripted fake in the tests) and handles
//! every exit the kernel hands back:
//!
//! | Exit | Handling |
//! | --- | --- |
//! | I/O (in/out, ins/outs, rep) | the port's device; unknown ports read all ones |
//! | EPT violation outside RAM | MMIO: fetch and decode the instruction, the device at that address, unknown addresses read all ones |
//! | CPUID | [`policy::cpuid`] over the host's own answer |
//! | RDMSR/WRMSR | [`policy::msr_kind`]: state, stored, TSC, or #GP |
//! | MOV CR0/3/4/8, CLTS, LMSW | the vCPU state (and EFER.LMA as the CPU sets it), TPR |
//! | MOV DR | reads give reset values, writes are dropped |
//! | HLT | with IF set: wait for the next interrupt; with IF clear: the guest is done |
//! | VMCALL, XSETBV, VMX instructions, MONITOR/MWAIT | #UD |
//! | host interrupt, time slice, host NMI | nothing: run again |
//! | triple fault, entry failure, anything else | stop, with the reason |
//!
//! Interrupts reach the guest through the PIC: before each entry, if a vector is
//! pending and the guest can take it (RFLAGS.IF, no STI/MOV SS shadow), it is injected.
//! The kernel has no interrupt-window exit in D1, so an interrupt that arrives while
//! the guest has IF clear waits for the next exit (the kernel's time slice bounds that).
use crate::memory::Ram;
use crate::pic::Pic;
use crate::pit::Pit;
use crate::policy::{self, MsrKind, MsrStore};
use crate::uart::{self, LineBuffer, Uart};
use crate::virtio::{Blk, Disk};
use crate::x86;
use sex_pdx::vm::{self, exit, Exit, State};

/// The kernel's `SYS_VM`, or a test double.
pub trait Vcpu {
    /// One entry; returns at the next exit (including host interrupts and time slices).
    fn run(&mut self, exit: &mut Exit, load_regs: bool) -> Result<(), u64>;
    fn get_state(&mut self, st: &mut State) -> Result<(), u64>;
    fn set_state(&mut self, st: &State) -> Result<(), u64>;
    fn inject(&mut self, vector: u8, kind: u32, error: Option<u32>) -> Result<(), u64>;
}

/// What the machine needs from the program around it.
pub trait Host {
    /// Monotonic time in nanoseconds.
    fn now_ns(&mut self) -> u64;
    fn tsc(&mut self) -> u64;
    fn cpuid(&mut self, leaf: u32, subleaf: u32) -> [u32; 4];
    /// A finished line from the guest's serial console (cleaned: printable ASCII).
    fn console(&mut self, line: &[u8]);
    /// Something worth a log line (bounded: the machine reports each kind a few times).
    fn note(&mut self, note: Note);
    /// The guest halted waiting for an interrupt that comes at `until` (ns) at the
    /// latest, or None for no timer. Wait a while; false stops the machine.
    fn idle(&mut self, until: Option<u64>) -> bool;
    /// Called on host interrupts and time slices; false stops the machine.
    fn keep_going(&mut self) -> bool { true }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Port { port: u16, write: bool },
    Mmio { gpa: u64, write: bool },
    Msr { index: u32, write: bool },
    Injected { vector: u8 },
}

/// Why the machine stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// HLT with interrupts off: the guest is done.
    Halted { rip: u64 },
    TripleFault { rip: u64 },
    /// VM entry refused the guest state (basic reason).
    EntryFailed { reason: u32 },
    /// The kernel refused a SYS_VM call (status).
    Hypervisor { status: u64 },
    /// An MMIO instruction Drag can not decode, or code fetched from MMIO.
    Mmio { gpa: u64, rip: u64, bytes: [u8; 15], len: u8 },
    /// A guest-physical address inside RAM faulted: the loan is gone.
    LostMemory { gpa: u64 },
    Unsupported { reason: u32, qualification: u64, rip: u64 },
    /// The host said stop, or the exit budget ran out.
    Host,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters { pub exits: u64, pub io: u64, pub mmio: u64, pub cpuid: u64, pub msr: u64, pub hlt: u64, pub injected: u64, pub other: u64 }

pub const IRQ_TIMER: u8 = 0;
const RFLAGS_IF: u64 = 1 << 9;
const RFLAGS_DF: u64 = 1 << 10;
const CR0_PG: u64 = 1 << 31;
const EFER_LME: u64 = 1 << 8;
const EFER_LMA: u64 = 1 << 10;
const VECTOR_UD: u8 = 6;
const VECTOR_GP: u8 = 13;
/// Most elements one string I/O exit moves before the instruction runs again.
const STRING_MAX: u64 = 1024;
/// How many times each kind of note is reported.
const NOTES_MAX: u32 = 16;

pub struct Machine<D: Disk> {
    pub ram: Ram,
    pub uart: Uart,
    pub pic: Pic,
    pub pit: Pit,
    pub blk: Option<Blk<D>>,
    pub counters: Counters,
    line: LineBuffer,
    debug_line: LineBuffer,
    msrs: MsrStore,
    tpr: u64,
    cmos_index: u8,
    notes: u32,
    /// An event went to the kernel and the guest has not entered since: the kernel
    /// holds one event per entry, so an interrupt waits for the next exit.
    queued: bool,
}

enum Flow { Go { load: bool, skipped: bool }, Stop(Stop) }
use Flow::Go;
const SAME: Flow = Go { load: false, skipped: false };
const SKIP: Flow = Go { load: true, skipped: true };

fn mask(size: u8) -> u64 { if size >= 8 { u64::MAX } else { (1u64 << (size as u32 * 8)) - 1 } }

impl<D: Disk> Machine<D> {
    pub fn new(ram: Ram, blk: Option<Blk<D>>) -> Machine<D> {
        Machine { ram, uart: Uart::new(), pic: Pic::new(), pit: Pit::new(), blk, counters: Counters::default(),
            line: LineBuffer::default(), debug_line: LineBuffer::default(), msrs: MsrStore::default(), tpr: 0, cmos_index: 0, notes: 0, queued: false }
    }

    /// Loads a bzImage the Linux way, with the virtio-blk device (if any) appended to
    /// the command line. Returns the entry state.
    pub fn load_linux(&mut self, image: &[u8], initrd: Option<&[u8]>, cmdline: &[u8]) -> Result<crate::boot::Boot, crate::boot::Error> {
        let mut line = alloc::vec::Vec::new();
        line.extend_from_slice(cmdline);
        if let Some(b) = &self.blk {
            if !line.is_empty() { line.push(b' '); }
            b.cmdline(&mut line);
        }
        crate::boot::load(&mut self.ram, image, initrd, &line)
    }

    fn note(&mut self, host: &mut impl Host, n: Note) {
        if self.notes < NOTES_MAX { self.notes += 1; host.note(n); }
    }

    /// Advances the timers and moves device interrupt lines to the PIC.
    fn tick(&mut self, now: u64) {
        if self.pit.poll(now) { self.pic.set_irq(IRQ_TIMER, true); self.pic.set_irq(IRQ_TIMER, false); }
        self.pic.set_irq(uart::IRQ, self.uart.irq());
        if let Some(b) = &self.blk { self.pic.set_irq(b.irq, b.irq_level()); }
    }

    /// Runs the vCPU until it stops, for at most `budget` exits.
    pub fn run(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, budget: u64) -> Stop {
        let mut exit = Exit::default();
        let (mut load, mut skipped) = (false, false);
        let mut fresh = true;
        loop {
            if self.counters.exits >= budget { return Stop::Host; }
            self.tick(host.now_ns());
            if !fresh && !self.queued && exit.regs.rflags & RFLAGS_IF != 0 && self.pic.pending().is_some() {
                match self.deliver(cpu, host, &mut exit, load, skipped) {
                    Ok(true) => load = false,
                    Ok(false) => {}
                    Err(status) => return Stop::Hypervisor { status },
                }
            }
            if let Err(status) = cpu.run(&mut exit, load) { return Stop::Hypervisor { status }; }
            fresh = false;
            self.queued = false;
            self.counters.exits += 1;
            match self.dispatch(cpu, host, &mut exit) {
                Go { load: l, skipped: s } => { load = l; skipped = s; }
                Flow::Stop(stop) => return stop,
            }
        }
    }

    /// Injects the PIC's pending vector if the guest can take it now. True when the
    /// state (and so the registers) went to the kernel.
    fn deliver(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, exit: &mut Exit, load: bool, skipped: bool) -> Result<bool, u64> {
        let mut st = State::default();
        cpu.get_state(&mut st)?;
        if load { st.regs = exit.regs; }
        // Stepping over the instruction that exited ends an STI or MOV SS shadow.
        if skipped { st.interruptibility &= !3; }
        if st.interruptibility & 3 != 0 { return Ok(false); }
        let Some(vector) = self.pic.pending() else { return Ok(false) };
        st.activity = 0;
        cpu.set_state(&st)?;
        cpu.inject(vector, vm::EVENT_EXTERNAL, None)?;
        // INTA only once the kernel holds the event, so a refusal loses no interrupt.
        self.pic.acknowledge();
        self.queued = true;
        exit.regs = st.regs;
        self.counters.injected += 1;
        if self.counters.injected <= 2 { self.note(host, Note::Injected { vector }); }
        Ok(true)
    }

    fn state(cpu: &mut impl Vcpu, exit: &Exit) -> Result<State, Flow> {
        let mut st = State::default();
        cpu.get_state(&mut st).map_err(|status| Flow::Stop(Stop::Hypervisor { status }))?;
        st.regs = exit.regs;
        Ok(st)
    }

    fn skip(exit: &mut Exit) { exit.regs.rip = exit.regs.rip.wrapping_add(exit.instruction_len as u64); }

    fn exception(&mut self, cpu: &mut impl Vcpu, vector: u8, error: Option<u32>) -> Flow {
        match cpu.inject(vector, vm::EVENT_EXCEPTION, error) {
            Ok(()) => { self.queued = true; SAME }
            Err(status) => Flow::Stop(Stop::Hypervisor { status }),
        }
    }

    fn dispatch(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, exit: &mut Exit) -> Flow {
        let rip = exit.regs.rip;
        if exit.reason & exit::ENTRY_FAILURE != 0 { return Flow::Stop(Stop::EntryFailed { reason: exit.reason & 0xFFFF }); }
        match exit.reason {
            exit::EXTERNAL_INTERRUPT | exit::PREEMPTED | exit::EXCEPTION_NMI => {
                if host.keep_going() { SAME } else { Flow::Stop(Stop::Host) }
            }
            exit::IO => { self.counters.io += 1; self.io(cpu, host, exit) }
            exit::EPT_VIOLATION => { self.counters.mmio += 1; self.mmio(cpu, host, exit) }
            exit::CPUID => {
                self.counters.cpuid += 1;
                let r = policy::cpuid(exit.regs.rax as u32, exit.regs.rcx as u32, |l, s| host.cpuid(l, s));
                exit.regs.rax = r[0] as u64; exit.regs.rbx = r[1] as u64; exit.regs.rcx = r[2] as u64; exit.regs.rdx = r[3] as u64;
                Self::skip(exit);
                SKIP
            }
            exit::RDMSR | exit::WRMSR => { self.counters.msr += 1; self.msr(cpu, host, exit) }
            exit::HLT => {
                self.counters.hlt += 1;
                Self::skip(exit);
                if exit.regs.rflags & RFLAGS_IF == 0 { return Flow::Stop(Stop::Halted { rip }); }
                loop {
                    let now = host.now_ns();
                    self.tick(now);
                    if self.pic.pending().is_some() { return SKIP; }
                    if !host.idle(self.pit.next_irq(now)) { return Flow::Stop(Stop::Host); }
                }
            }
            exit::CR_ACCESS => self.cr(cpu, exit),
            29 => {
                // MOV DR: reads see reset values, writes are dropped.
                let q = exit.qualification;
                if q & 0x10 != 0 {
                    let v = match q & 7 { 6 => 0xFFFF_0FF0, 7 => 0x400, _ => 0 };
                    *x86::gpr(&mut exit.regs, ((q >> 8) & 15) as u8) = v;
                }
                Self::skip(exit);
                SKIP
            }
            13 | 54 => { Self::skip(exit); SKIP } // INVD, WBINVD
            exit::VMCALL | 19..=27 | 36 | 39 | 50 | 53 | exit::XSETBV | 58 => self.exception(cpu, VECTOR_UD, None),
            15 => self.exception(cpu, VECTOR_GP, Some(0)), // RDPMC
            exit::TRIPLE_FAULT => Flow::Stop(Stop::TripleFault { rip }),
            reason => { self.counters.other += 1; Flow::Stop(Stop::Unsupported { reason, qualification: exit.qualification, rip }) }
        }
    }

    fn port_read(&mut self, host: &mut impl Host, port: u16, now: u64) -> u8 {
        match port {
            0x3F8..=0x3FF => self.uart.read(port - uart::COM1),
            p if Pic::handles(p) => self.pic.read(p),
            p if Pit::handles(p) => self.pit.read(p, now),
            0x71 => match self.cmos_index & 0x7F {
                0x0A => 0x26, 0x0B => 0x02, 0x0D => 0x80, 0x07 | 0x08 => 1, 0x32 => 0x20, _ => 0,
            },
            0x70 => self.cmos_index,
            0x92 => 0x02, // A20 on
            0xCF8..=0xCFB => 0, // no PCI configuration mechanism
            0x80 | 0xE9 => 0,
            _ => { self.note(host, Note::Port { port, write: false }); 0xFF }
        }
    }

    fn port_write(&mut self, host: &mut impl Host, port: u16, v: u8, now: u64) {
        match port {
            0x3F8..=0x3FF => {
                if let Some(b) = self.uart.write(port - uart::COM1, v) {
                    if let Some(line) = self.line.push(b) { host.console(line); }
                }
            }
            0xE9 => if let Some(line) = self.debug_line.push(v) { host.console(line); },
            p if Pic::handles(p) => self.pic.write(p, v),
            p if Pit::handles(p) => self.pit.write(p, v, now),
            0x70 => self.cmos_index = v,
            0x71 | 0x80 | 0x92 | 0xCF8..=0xCFF => {}
            _ => self.note(host, Note::Port { port, write: true }),
        }
    }

    fn port(&mut self, host: &mut impl Host, port: u16, size: u8, write: bool, value: u64) -> u64 {
        let now = host.now_ns();
        let mut out = 0u64;
        for i in 0..size.min(4) as u16 {
            let p = port.wrapping_add(i);
            if write { self.port_write(host, p, (value >> (8 * i)) as u8, now) } else { out |= (self.port_read(host, p, now) as u64) << (8 * i) }
        }
        out
    }

    fn io(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, exit: &mut Exit) -> Flow {
        let (port, size, flags) = (exit.port, exit.io_size.clamp(1, 4), exit.io_flags);
        let input = flags & vm::IO_IN != 0;
        if flags & vm::IO_STRING == 0 {
            if input {
                let v = self.port(host, port, size, false, 0);
                let r = &mut exit.regs.rax;
                *r = if size == 4 { v } else { (*r & !mask(size)) | v };
            } else {
                self.port(host, port, size, true, exit.regs.rax & mask(size));
            }
            Self::skip(exit);
            return SKIP;
        }
        // ins/outs: the first element is at the exit's guest-linear address; the
        // address size (instruction info bits 9:7) bounds RCX, RSI and RDI.
        let st = match Self::state(cpu, exit) { Ok(s) => s, Err(f) => return f };
        let amask = match (exit.instruction_info >> 7) & 7 { 0 => 0xFFFF, 1 => 0xFFFF_FFFF, _ => u64::MAX };
        let rep = flags & vm::IO_REP != 0;
        let count = if rep { exit.regs.rcx & amask } else { 1 };
        let n = count.min(STRING_MAX);
        let step = if exit.regs.rflags & RFLAGS_DF != 0 { (size as u64).wrapping_neg() } else { size as u64 };
        let mut linear = exit.guest_linear;
        let mut done = 0;
        while done < n {
            let mut buf = [0u8; 4];
            let b = &mut buf[..size as usize];
            let ok = if input {
                let v = self.port(host, port, size, false, 0);
                b.copy_from_slice(&v.to_le_bytes()[..size as usize]);
                x86::write_linear(&mut self.ram, &st, linear, b).is_some()
            } else {
                x86::read_linear(&self.ram, &st, linear, b).is_some().then(|| { self.port(host, port, size, true, u32::from_le_bytes(buf) as u64); }).is_some()
            };
            if !ok { break; } // an unmapped page: the guest faults on its own retry
            linear = linear.wrapping_add(step);
            done += 1;
        }
        let r = &mut exit.regs;
        let index = if input { &mut r.rdi } else { &mut r.rsi };
        *index = (*index & !amask) | (index.wrapping_add(step.wrapping_mul(done)) & amask);
        if rep { r.rcx = (r.rcx & !amask) | (r.rcx.wrapping_sub(done) & amask); }
        if !rep || r.rcx & amask == 0 { Self::skip(exit); }
        SKIP
    }

    fn mmio(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, exit: &mut Exit) -> Flow {
        let gpa = exit.guest_physical;
        let rip = exit.regs.rip;
        if gpa < self.ram.len() { return Flow::Stop(Stop::LostMemory { gpa }); }
        let st = match Self::state(cpu, exit) { Ok(s) => s, Err(f) => return f };
        let mut bytes = [0u8; 15];
        let len = x86::fetch(&self.ram, &st, &mut bytes);
        let fetch_from_mmio = exit.qualification & 4 != 0;
        let Some(m) = (!fetch_from_mmio).then(|| x86::decode(&bytes[..len], x86::long_mode(&st), x86::default32(&st))).flatten() else {
            return Flow::Stop(Stop::Mmio { gpa, rip, bytes, len: len as u8 });
        };
        let write = !matches!(m.op, x86::Op::Load { .. });
        let dev = self.blk.as_mut().filter(|b| b.contains(gpa));
        match dev {
            Some(b) => {
                let offset = gpa - b.base;
                if write {
                    let v = x86::store_value(&mut exit.regs, &m);
                    b.write(&mut self.ram, offset, m.size, v);
                } else {
                    let v = b.read(offset, m.size);
                    x86::complete_load(&mut exit.regs, &m, v);
                }
            }
            None => {
                self.note(host, Note::Mmio { gpa, write });
                if !write { x86::complete_load(&mut exit.regs, &m, u64::MAX); }
            }
        }
        exit.regs.rip = exit.regs.rip.wrapping_add(m.len as u64);
        SKIP
    }

    fn msr(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, exit: &mut Exit) -> Flow {
        let index = exit.regs.rcx as u32;
        let write = exit.reason == exit::WRMSR;
        let value = (exit.regs.rdx << 32) | (exit.regs.rax & 0xFFFF_FFFF);
        let read = match policy::msr_kind(index) {
            MsrKind::Absent => {
                self.note(host, Note::Msr { index, write });
                return self.exception(cpu, VECTOR_GP, Some(0));
            }
            MsrKind::Tsc => host.tsc(),
            MsrKind::Stored(read_only) => {
                if write && !read_only { self.msrs.write(index, value); }
                self.msrs.read(index)
            }
            MsrKind::State => {
                let mut st = match Self::state(cpu, exit) { Ok(s) => s, Err(f) => return f };
                let slot = match index { policy::msr::FS_BASE => Some(4), policy::msr::GS_BASE => Some(5), _ => None };
                let current = match slot { Some(i) => st.segments[i].base, None => st.efer };
                if write {
                    match slot {
                        Some(i) => st.segments[i].base = value,
                        None => match policy::efer_write(st.efer, value) {
                            Ok(v) => st.efer = v,
                            Err(()) => return self.exception(cpu, VECTOR_GP, Some(0)),
                        },
                    }
                    Self::skip(exit);
                    st.regs = exit.regs;
                    if let Err(status) = cpu.set_state(&st) { return Flow::Stop(Stop::Hypervisor { status }); }
                    return Go { load: false, skipped: true };
                }
                current
            }
        };
        if !write { exit.regs.rax = read & 0xFFFF_FFFF; exit.regs.rdx = read >> 32; }
        Self::skip(exit);
        SKIP
    }

    fn cr(&mut self, cpu: &mut impl Vcpu, exit: &mut Exit) -> Flow {
        let q = exit.qualification;
        let (cr, access, reg) = (q & 15, (q >> 4) & 3, ((q >> 8) & 15) as u8);
        if cr == 8 && access < 2 {
            if access == 0 { self.tpr = *x86::gpr(&mut exit.regs, reg) & 15 } else { *x86::gpr(&mut exit.regs, reg) = self.tpr }
            Self::skip(exit);
            return SKIP;
        }
        let mut st = match Self::state(cpu, exit) { Ok(s) => s, Err(f) => return f };
        match (access, cr) {
            (0, 0) => {
                let value = *x86::gpr(&mut st.regs, reg);
                set_cr0(&mut st, value);
            }
            (0, 3) => st.guest_cr3 = *x86::gpr(&mut st.regs, reg),
            (0, 4) => st.cr4 = *x86::gpr(&mut st.regs, reg),
            (1, 0) => *x86::gpr(&mut st.regs, reg) = st.cr0,
            (1, 3) => *x86::gpr(&mut st.regs, reg) = st.guest_cr3,
            (1, 4) => *x86::gpr(&mut st.regs, reg) = st.cr4,
            (2, _) => st.cr0 &= !8, // CLTS
            (3, _) => {
                // LMSW: the low four bits of CR0; PE can be set, not cleared.
                let src = (q >> 16) & 0xF;
                let cr0 = (st.cr0 & !0xE) | src | (st.cr0 & 1);
                set_cr0(&mut st, cr0);
            }
            _ => return Flow::Stop(Stop::Unsupported { reason: exit::CR_ACCESS, qualification: q, rip: exit.regs.rip }),
        }
        st.regs.rip = st.regs.rip.wrapping_add(exit.instruction_len as u64);
        if let Err(status) = cpu.set_state(&st) { return Flow::Stop(Stop::Hypervisor { status }); }
        exit.regs = st.regs;
        Go { load: false, skipped: true }
    }

    /// Guest serial output not yet ended by a newline.
    pub fn console_pending(&self) -> &[u8] { self.line.pending() }
}

/// CR0 as the CPU takes it, with EFER.LMA following paging when LME is set.
fn set_cr0(st: &mut State, value: u64) {
    let was = st.cr0 & CR0_PG != 0;
    st.cr0 = value;
    let now = value & CR0_PG != 0;
    if now && !was && st.efer & EFER_LME != 0 { st.efer |= EFER_LMA; }
    if !now { st.efer &= !EFER_LMA; }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::virtio::MemDisk;
    use alloc::vec::Vec;
    use alloc::vec;

    /// A vCPU that plays back a script of exits and records what the VMM did.
    #[derive(Default)]
    pub struct Script {
        pub exits: Vec<Exit>,
        pub state: State,
        pub loads: Vec<(bool, sex_pdx::vm::Regs)>,
        pub injected: Vec<(u8, u32, Option<u32>)>,
        /// RIP when each event was injected.
        pub injected_at: Vec<u64>,
        pub sets: u32,
        /// An event is waiting for the next entry (the kernel answers BUSY to a second).
        pub event: bool,
    }
    impl Vcpu for Script {
        fn run(&mut self, exit: &mut Exit, load: bool) -> Result<(), u64> {
            self.loads.push((load, exit.regs));
            self.event = false;
            if load { self.state.regs = exit.regs; }
            if self.exits.is_empty() { return Err(99); }
            let mut next = self.exits.remove(0);
            // Unless the script says otherwise, the guest continues from where the VMM left it.
            if next.regs == Default::default() { next.regs = self.state.regs; }
            self.state.regs = next.regs;
            *exit = next;
            Ok(())
        }
        fn get_state(&mut self, st: &mut State) -> Result<(), u64> { *st = self.state; Ok(()) }
        fn set_state(&mut self, st: &State) -> Result<(), u64> { self.state = *st; self.sets += 1; Ok(()) }
        fn inject(&mut self, vector: u8, kind: u32, error: Option<u32>) -> Result<(), u64> {
            if self.event { return Err(5); }
            self.event = true;
            self.injected.push((vector, kind, error)); self.injected_at.push(self.state.regs.rip); Ok(())
        }
    }

    #[derive(Default)]
    pub struct Log { pub lines: Vec<Vec<u8>>, pub notes: Vec<Note>, pub now: u64, pub idles: u32 }
    impl Host for Log {
        fn now_ns(&mut self) -> u64 { self.now }
        fn tsc(&mut self) -> u64 { 12345 }
        fn cpuid(&mut self, _: u32, _: u32) -> [u32; 4] { [u32::MAX; 4] }
        fn console(&mut self, line: &[u8]) { self.lines.push(line.to_vec()); }
        fn note(&mut self, note: Note) { self.notes.push(note); }
        fn idle(&mut self, until: Option<u64>) -> bool {
            self.idles += 1;
            match until { Some(t) if self.idles < 100 => { self.now = t; true } _ => false }
        }
    }

    pub fn io(port: u16, size: u8, input: bool, rax: u64) -> Exit {
        let mut e = Exit { reason: exit::IO, instruction_len: 1, port, io_size: size, io_flags: if input { vm::IO_IN } else { 0 }, ..Exit::default() };
        e.regs.rax = rax; e.regs.rip = 0x1000; e.regs.rflags = 2;
        e
    }
    fn plain(reason: u32, len: u32) -> Exit { Exit { reason, instruction_len: len, ..Exit::default() } }

    fn machine() -> Machine<MemDisk> {
        let blk = Blk::new(crate::virtio::tests::disk(8), 0xD000_0000, 5);
        Machine::new(Ram::new(4 << 20).unwrap(), Some(blk))
    }

    #[test]
    fn serial_console_lines_and_halt() {
        let mut m = machine();
        let mut cpu = Script::default();
        for &b in b"hello\n" { cpu.exits.push(io(0x3F8, 1, false, b as u64)); }
        cpu.exits.push(io(0x3FD, 1, true, 0xFFFF_FFFF_FFFF_FF00));
        cpu.exits.push(plain(exit::HLT, 1));
        let mut log = Log::default();
        let stop = m.run(&mut cpu, &mut log, 100);
        assert_eq!(stop, Stop::Halted { rip: 0x1001 });
        assert_eq!(log.lines, [b"hello".to_vec()]);
        // The LSR read put 0x60 in AL and kept the rest of RAX; RIP stepped past each OUT.
        let (load, regs) = cpu.loads[7];
        assert!(load);
        assert_eq!(regs.rax, 0xFFFF_FFFF_FFFF_FF60);
        assert_eq!(regs.rip, 0x1001);
        assert_eq!(m.counters.io, 7);
    }

    #[test]
    fn unknown_ports_read_all_ones_and_are_noted_a_few_times() {
        let mut m = machine();
        let mut cpu = Script::default();
        for _ in 0..40 { cpu.exits.push(io(0x1234, 2, true, 0)); }
        cpu.exits.push(io(0x1300, 4, true, u64::MAX));
        let mut log = Log::default();
        assert_eq!(m.run(&mut cpu, &mut log, 1000), Stop::Hypervisor { status: 99 });
        assert_eq!(cpu.loads[1].1.rax & 0xFFFF, 0xFFFF);
        assert_eq!(cpu.loads.last().unwrap().1.rax, 0xFFFF_FFFF, "a 32-bit IN zero-extends");
        assert_eq!(log.notes.len(), NOTES_MAX as usize);
    }

    #[test]
    fn cpuid_msr_and_control_registers() {
        let mut m = machine();
        let mut cpu = Script { state: crate::boot::long_mode(0x10_0200), ..Script::default() };
        let mut e = plain(exit::CPUID, 2);
        e.regs = cpu.state.regs; e.regs.rax = 1;
        cpu.exits.push(e);
        // wrmsr LSTAR, rdmsr LSTAR, rdmsr SPEC_CTRL (#GP), wrmsr EFER |= NXE, wrmsr GS_BASE.
        let msr = |reason, ecx, rdx, rax| { let mut e = plain(reason, 2); e.regs.rip = 0x2000; e.regs.rcx = ecx; e.regs.rdx = rdx; e.regs.rax = rax; e };
        cpu.exits.push(msr(exit::WRMSR, 0xC000_0082, 0xFFFF_FFFF, 0x8100_0000));
        cpu.exits.push(msr(exit::RDMSR, 0xC000_0082, 0, 0));
        cpu.exits.push(msr(exit::RDMSR, 0x48, 0, 0));
        cpu.exits.push(msr(exit::WRMSR, 0xC000_0080, 0, 0xD01));
        cpu.exits.push(msr(exit::WRMSR, 0xC000_0101, 0x1, 0x2345_0000));
        cpu.exits.push(msr(exit::WRMSR, 0xC000_0080, 0, 1 << 12));
        // mov %rax,%cr8; mov %cr8,%rbx; CLTS
        let mut cr8 = plain(exit::CR_ACCESS, 4);
        cr8.qualification = 8; cr8.regs.rax = 0xA; cr8.regs.rip = 0x3000;
        cpu.exits.push(cr8);
        let mut cr8r = plain(exit::CR_ACCESS, 4);
        cr8r.qualification = 8 | 1 << 4 | 3 << 8; cr8r.regs.rip = 0x3004;
        cpu.exits.push(cr8r);
        let mut log = Log::default();
        m.run(&mut cpu, &mut log, 100);
        let regs = |i: usize| cpu.loads[i].1;
        assert_eq!(regs(1).rcx >> 31, 1, "hypervisor bit in CPUID.1:ECX");
        assert_eq!(regs(1).rip, 0x10_0202);
        assert_eq!((regs(3).rax, regs(3).rdx), (0x8100_0000, 0xFFFF_FFFF), "LSTAR read back");
        assert_eq!(cpu.injected, [(VECTOR_GP, vm::EVENT_EXCEPTION, Some(0)), (VECTOR_GP, vm::EVENT_EXCEPTION, Some(0))]);
        assert!(!cpu.loads[4].0, "a #GP leaves RIP alone");
        assert_eq!(cpu.state.efer, 0xD00 | 1, "EFER written through the state, LMA kept");
        assert_eq!(cpu.state.segments[5].base, 0x1_2345_0000);
        assert_eq!(m.tpr, 0xA);
        assert_eq!(cpu.loads[9].1.rbx, 0xA);
    }

    #[test]
    fn cr0_paging_sets_lma() {
        let mut st = crate::boot::protected_mode(0);
        st.efer = EFER_LME;
        set_cr0(&mut st, 0x8000_0011);
        assert_eq!(st.efer, EFER_LME | EFER_LMA);
        set_cr0(&mut st, 0x11);
        assert_eq!(st.efer, EFER_LME);
    }

    #[test]
    fn timer_interrupt_wakes_a_halted_guest() {
        let mut m = machine();
        let mut cpu = Script::default();
        cpu.state.regs.rflags = 2 | RFLAGS_IF;
        // Linux-style: program the PIC (vector 0x30, only IRQ 0) and the PIT at 1 kHz.
        for (port, v) in [(0x20, 0x11), (0x21, 0x30), (0x21, 0x04), (0x21, 0x01), (0x21, 0xFE), (0x43, 0x34), (0x40, 0xA9), (0x40, 0x04)] {
            let mut e = io(port, 1, false, v);
            e.regs.rflags = 2 | RFLAGS_IF;
            cpu.exits.push(e);
        }
        let mut hlt = plain(exit::HLT, 1);
        hlt.regs.rflags = 2 | RFLAGS_IF; hlt.regs.rip = 0x5000;
        cpu.exits.push(hlt);
        let mut sti_shadow = plain(exit::CPUID, 2);
        sti_shadow.regs.rflags = 2 | RFLAGS_IF;
        cpu.exits.push(sti_shadow);
        let mut log = Log::default();
        m.run(&mut cpu, &mut log, 100);
        assert_eq!(cpu.injected.first(), Some(&(0x30, vm::EVENT_EXTERNAL, None)));
        assert!(log.idles >= 1, "the halted guest waited for the PIT");
        assert_eq!(cpu.injected_at[0], 0x5001, "injected after HLT, which was stepped over");
        assert_eq!(m.counters.injected, 1, "IRQ 0 stays in service until EOI");
    }

    #[test]
    fn interrupt_waits_for_a_queued_exception() {
        let mut m = machine();
        let mut cpu = Script::default();
        cpu.state.regs.rflags = 2 | RFLAGS_IF;
        for (port, v) in [(0x20, 0x11), (0x21, 0x30), (0x21, 0x04), (0x21, 0x01), (0x21, 0xFE)] {
            let mut e = io(port, 1, false, v);
            e.regs.rflags = 2 | RFLAGS_IF;
            cpu.exits.push(e);
        }
        // An unsupported RDMSR (#GP) while IRQ 0 is raised.
        let mut rd = plain(exit::RDMSR, 2);
        rd.regs.rflags = 2 | RFLAGS_IF; rd.regs.rip = 0x2000; rd.regs.rcx = 0x48;
        cpu.exits.push(rd);
        let mut next = plain(exit::CPUID, 2);
        next.regs.rflags = 2 | RFLAGS_IF; next.regs.rip = 0x3000;
        cpu.exits.push(next);
        let mut log = Log::default();
        assert_eq!(m.run(&mut cpu, &mut log, 5), Stop::Host, "PIC programmed");
        m.pic.set_irq(0, true); m.pic.set_irq(0, false);
        let stop = m.run(&mut cpu, &mut log, 100);
        assert!(!matches!(stop, Stop::Hypervisor { status: 5 }), "no BUSY stop: {stop:?}");
        assert_eq!(cpu.injected.iter().map(|e| (e.0, e.1)).collect::<Vec<_>>(),
            [(VECTOR_GP, vm::EVENT_EXCEPTION), (0x30, vm::EVENT_EXTERNAL)], "#GP first, the interrupt on the next entry");
        assert_eq!(m.counters.injected, 1);
    }

    #[test]
    fn no_injection_in_an_interrupt_shadow_or_with_if_clear() {
        let mut m = machine();
        m.pic.write(0x20, 0x11); m.pic.write(0x21, 0x30); m.pic.write(0x21, 4); m.pic.write(0x21, 1); m.pic.write(0x21, 0);
        m.pic.set_irq(1, true);
        let mut cpu = Script::default();
        cpu.state.interruptibility = 1; // STI shadow on a time-slice exit
        let mut e = plain(exit::PREEMPTED, 0);
        e.regs.rflags = 2 | RFLAGS_IF;
        cpu.exits.push(e);
        let mut off = plain(exit::PREEMPTED, 0);
        off.regs.rflags = 2;
        cpu.exits.push(off);
        let mut log = Log::default();
        m.run(&mut cpu, &mut log, 100);
        assert!(cpu.injected.is_empty());
    }

    #[test]
    fn string_io_rep_outsb() {
        let mut m = machine();
        let mut cpu = Script { state: crate::boot::long_mode(0), ..Script::default() };
        crate::boot::load(&mut m.ram, &crate::boot::tests::bzimage(0x020F, 2, 1, &[0; 16]), None, b"").unwrap();
        m.ram.write(0x30000, b"rep out\n").unwrap();
        let mut e = io(0x3F8, 1, false, 0);
        e.io_flags = vm::IO_STRING | vm::IO_REP;
        e.guest_linear = 0x30000;
        e.instruction_info = 2 << 7;
        e.regs.rsi = 0x30000; e.regs.rcx = 8; e.instruction_len = 2;
        cpu.exits.push(e);
        let mut log = Log::default();
        m.run(&mut cpu, &mut log, 10);
        assert_eq!(log.lines, [b"rep out".to_vec()]);
        let r = cpu.loads[1].1;
        assert_eq!((r.rsi, r.rcx, r.rip), (0x30008, 0, 0x1002));
    }

    #[test]
    fn hostile_exits_stop_cleanly() {
        let mut m = machine();
        let mut cpu = Script::default();
        cpu.exits.push(plain(exit::TRIPLE_FAULT, 0));
        assert!(matches!(m.run(&mut cpu, &mut Log::default(), 10), Stop::TripleFault { .. }));
        cpu.exits.push(Exit { reason: exit::ENTRY_FAILURE | 33, ..Exit::default() });
        assert_eq!(m.run(&mut cpu, &mut Log::default(), 10), Stop::EntryFailed { reason: 33 });
        cpu.exits.push(plain(0xFFFF, 0));
        assert!(matches!(m.run(&mut cpu, &mut Log::default(), 10), Stop::Unsupported { reason: 0xFFFF, .. }));
        cpu.exits.push(Exit { reason: exit::EPT_VIOLATION, guest_physical: 0x1000, ..Exit::default() });
        assert_eq!(m.run(&mut cpu, &mut Log::default(), 10), Stop::LostMemory { gpa: 0x1000 });
        // A garbage I/O size and a huge REP count are bounded.
        let mut e = io(0x3F8, 200, false, 0);
        e.io_flags = vm::IO_STRING | vm::IO_REP;
        e.regs.rcx = u64::MAX;
        e.guest_linear = u64::MAX - 1;
        cpu.exits.push(e);
        cpu.exits.push(plain(exit::VMCALL, 3));
        m.run(&mut cpu, &mut Log::default(), 100);
        assert_eq!(cpu.injected.last(), Some(&(VECTOR_UD, vm::EVENT_EXCEPTION, None)));
        // The budget ends a guest that never stops.
        for _ in 0..50 { cpu.exits.push(plain(exit::PREEMPTED, 0)); }
        let before = m.counters.exits;
        assert_eq!(m.run(&mut cpu, &mut Log::default(), before + 10), Stop::Host);
    }

    /// The test guest's virtio-blk conversation, as MMIO exits.
    #[test]
    fn mmio_virtio_blk_through_decoded_instructions() {
        let mut m = machine();
        let mut cpu = Script { state: crate::boot::long_mode(0x10_0200), ..Script::default() };
        crate::boot::load(&mut m.ram, &crate::boot::tests::bzimage(0x020F, 2, 1, &[0; 16]), None, b"").unwrap();
        let base = 0xD000_0000u64;
        let mut at = 0x10_1000u64;
        let mut push = |cpu: &mut Script, ram: &mut Ram, code: &[u8], gpa: u64, rax: u64| {
            ram.write(at, code).unwrap();
            let mut e = Exit { reason: exit::EPT_VIOLATION, guest_physical: gpa, ..Exit::default() };
            e.regs = cpu.state.regs; e.regs.rip = at; e.regs.rax = rax; e.regs.rbx = base;
            cpu.exits.push(e);
            at += 0x10;
        };
        // mov 0x0(%rbx),%eax (magic); movl $imm,0x70(%rbx) ...; mov %eax,0x50(%rbx) (notify)
        push(&mut cpu, &mut m.ram, &[0x8B, 0x03], base, 0);
        let store = |off: u8, v: u32| { let mut c = vec![0xC7, 0x43, off]; c.extend_from_slice(&v.to_le_bytes()); c };
        crate::virtio::tests::desc(&mut m.ram, 0, 0x20000, 16, 1, 1);
        crate::virtio::tests::desc(&mut m.ram, 1, 0x21000, 512, 3, 2);
        crate::virtio::tests::desc(&mut m.ram, 2, 0x22000, 1, 2, 0);
        crate::virtio::tests::offer(&mut m.ram, 0);
        m.ram.write_u32(0x20000, 0).unwrap();
        m.ram.write_u64(0x20008, 2).unwrap();
        for (off, v) in [(0x70u8, 0u32), (0x70, 1), (0x70, 3), (0x24, 1), (0x20, 1), (0x70, 11), (0x38, 8), (0x30, 0)] {
            push(&mut cpu, &mut m.ram, &store(off, v), base + off as u64, 0);
        }
        for (off, v) in [(0x80u32, crate::virtio::tests::Q as u32), (0x90, crate::virtio::tests::Q as u32 + 0x100), (0xA0, crate::virtio::tests::Q as u32 + 0x1000), (0x44, 1), (0x70, 15)] {
            let mut c = vec![0xC7, 0x83];
            c.extend_from_slice(&off.to_le_bytes());
            c.extend_from_slice(&v.to_le_bytes());
            push(&mut cpu, &mut m.ram, &c, base + off as u64, 0);
        }
        push(&mut cpu, &mut m.ram, &[0x89, 0x43, 0x50], base + 0x50, 0);
        push(&mut cpu, &mut m.ram, &[0x0F, 0xB6, 0x83, 0, 1, 0, 0], base + 0x100, u64::MAX);
        push(&mut cpu, &mut m.ram, &[0x8B, 0x05, 0, 0, 0, 0], 0xFEE0_0000, 0); // an unknown device
        push(&mut cpu, &mut m.ram, &[0x0F, 0x0B], base, 0); // ud2 touching MMIO
        let mut log = Log::default();
        let stop = m.run(&mut cpu, &mut log, 100);
        assert!(matches!(stop, Stop::Mmio { len: 15, .. }), "{stop:?}");
        assert_eq!(cpu.loads[1].1.rax, crate::virtio::MAGIC as u64);
        assert_eq!(m.ram.read_u8(0x22000), Ok(0), "the read was served");
        assert_eq!(m.ram.slice(0x21000, 2).unwrap(), &[2, b'#']);
        let n = cpu.loads.len();
        assert_eq!(cpu.loads[n - 2].1.rax, 8, "movzx of the capacity's low byte");
        assert_eq!(cpu.loads[n - 1].1.rax, 0xFFFF_FFFF, "unknown MMIO reads all ones");
        assert!(log.notes.contains(&Note::Mmio { gpa: 0xFEE0_0000, write: false }));
        assert!(m.pic.pending().is_none(), "IRQ 5 is masked until the guest unmasks it");
        assert!(m.blk.as_ref().unwrap().irq_level());
    }
}
