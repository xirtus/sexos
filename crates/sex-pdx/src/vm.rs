// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Drag virtual machines (syscall 110, `SYS_VM`): the kernel's VM mechanism, phase D1.
//!
//! A program holding the Machine capability ([`SLOT`]) makes a VM, lends it pages of its
//! own heap at guest-physical addresses (the kernel maps them into the VM's EPT and
//! refuses anything else), makes a vCPU and runs it until the next VM exit. The exit
//! comes back to the program as an [`Exit`] record; I/O, MMIO, CPUID, MSRs, HLT and
//! every device are the program's job (the `drag` server in D2). The kernel keeps only
//! what ring 3 cannot do: VMX on/off, the VMCS, EPT, entry and exit.
//!
//! Intel VT-x only in D1 (AMD SVM is D5). VMX is entered inside `OP_VCPU_RUN` on the
//! calling core and left before the call returns, so a program that never calls this
//! leaves VMX untouched. Without usable VMX (QEMU TCG, AMD, firmware lock) every op but
//! [`OP_INFO`] answers [`UNAVAILABLE`] and `OP_INFO` says why.
//!
//! Registers: rax = 110, rdi = op, rsi/rdx/r10/r8 = arguments.
//! Reply: rax = status, rsi/rdx = values.

pub const SYS_VM: u64 = 110;
/// The Machine capability. BOOT grants it to the program rows only: no service, Browser,
/// Quil or Spindle run can make a VM.
pub const SLOT: u64 = 261;

/// rsi = 1 when VMX can run guests on this machine, rdx = a [`Reason`] code (0 = usable).
pub const OP_INFO: u64 = 0;
/// rsi = the new VM.
pub const OP_CREATE: u64 = 1;
/// rsi = vm, rdx = the caller's heap address, r10 = guest-physical address,
/// r8 = pages | access << 32 (access: [`READ`] [`WRITE`] [`EXEC`], READ required).
pub const OP_LEND_MEMORY: u64 = 2;
/// The same op: lending maps the pages into the VM.
pub const OP_MAP: u64 = OP_LEND_MEMORY;
/// rsi = vm. Reply rsi = the vCPU, which starts in real mode at 0000:0000.
pub const OP_VCPU_CREATE: u64 = 3;
/// rsi = vm, rdx = vcpu, r10 = address of an [`Exit`], r8 = [`RUN_LOAD_REGS`] or 0.
/// Runs until the next exit and writes the record there.
pub const OP_VCPU_RUN: u64 = 4;
/// rsi = vm, rdx = vcpu, r10 = address of a [`State`] to fill.
pub const OP_VCPU_GET_REGS: u64 = 5;
/// rsi = vm, rdx = vcpu, r10 = address of a [`State`] to load.
pub const OP_VCPU_SET_REGS: u64 = 6;
/// rsi = vm, rdx = vcpu, r10 = vector | kind << 8 | has_error << 11 (kinds [`EVENT_EXTERNAL`],
/// [`EVENT_NMI`], [`EVENT_EXCEPTION`]), r8 = error code. Delivered at the next entry.
pub const OP_INJECT: u64 = 7;
/// rsi = vm. Ends the VM; its pages stay the caller's.
pub const OP_DESTROY: u64 = 8;

/// `OP_VCPU_RUN` flag: first load `Exit::regs` (as the caller changed them) into the vCPU.
pub const RUN_LOAD_REGS: u64 = 1;

pub const READ: u64 = 1;
pub const WRITE: u64 = 2;
pub const EXEC: u64 = 4;

pub const EVENT_EXTERNAL: u32 = 0;
pub const EVENT_NMI: u32 = 2;
pub const EVENT_EXCEPTION: u32 = 3;

pub const OK: u64 = 0;
/// A bad op, argument or address.
pub const INVALID: u64 = 1;
/// No Machine capability, not the caller's VM, or memory the caller does not own.
pub const DENIED: u64 = 2;
/// No usable VMX on this machine (see `OP_INFO`).
pub const UNAVAILABLE: u64 = 3;
/// Too many VMs or vCPUs, or no memory for tables.
pub const EXHAUSTED: u64 = 4;
/// The vCPU is running, or an event is already queued.
pub const BUSY: u64 = 5;
/// VMLAUNCH refused the VMCS; rsi = the VM-instruction error number.
pub const ENTRY_FAILED: u64 = 6;

/// Why VMX is unavailable (`OP_INFO` rdx; also the kernel's `[drag] vmx=unavailable` line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Reason { Usable = 0, NoVmx = 1, FirmwareLocked = 2, Controls = 3, Ept = 4, Basic = 5 }
impl Reason {
    pub fn name(self) -> &'static str {
        match self { Reason::Usable => "usable", Reason::NoVmx => "no_vmx_cpuid", Reason::FirmwareLocked => "feature_control_locked",
            Reason::Controls => "missing_controls", Reason::Ept => "missing_ept", Reason::Basic => "vmx_basic" }
    }
}

/// Basic exit reasons (Intel SDM appendix C) the program sees most.
pub mod exit {
    pub const EXCEPTION_NMI: u32 = 0;
    /// A host interrupt arrived; the host handled it. Run again.
    pub const EXTERNAL_INTERRUPT: u32 = 1;
    pub const TRIPLE_FAULT: u32 = 2;
    pub const CPUID: u32 = 10;
    pub const HLT: u32 = 12;
    pub const VMCALL: u32 = 18;
    pub const CR_ACCESS: u32 = 28;
    pub const IO: u32 = 30;
    pub const RDMSR: u32 = 31;
    pub const WRMSR: u32 = 32;
    pub const EPT_VIOLATION: u32 = 48;
    pub const EPT_MISCONFIG: u32 = 49;
    /// The vCPU's time slice ended (fairness). Run again.
    pub const PREEMPTED: u32 = 52;
    pub const XSETBV: u32 = 55;
    /// Bit 31 of `Exit::reason`: the entry failed on guest state (reason 33/34).
    pub const ENTRY_FAILURE: u32 = 1 << 31;
}

/// `Exit::io_flags` bits.
pub const IO_IN: u8 = 1;
pub const IO_STRING: u8 = 2;
pub const IO_REP: u8 = 4;

/// The guest's general registers. The first fifteen are in the order the kernel's entry
/// code loads and saves them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64, pub rsi: u64, pub rdi: u64, pub rbp: u64,
    pub r8: u64, pub r9: u64, pub r10: u64, pub r11: u64, pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rsp: u64, pub rip: u64, pub rflags: u64,
}

/// One VM exit, as `OP_VCPU_RUN` writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exit {
    /// Basic exit reason ([`exit`]), with [`exit::ENTRY_FAILURE`] when entry failed.
    pub reason: u32,
    /// Length of the instruction that exited (advance `regs.rip` by it to skip it).
    pub instruction_len: u32,
    pub qualification: u64,
    pub guest_physical: u64,
    pub guest_linear: u64,
    pub instruction_info: u32,
    /// Event that exited (exceptions, NMI), as the VMCS gives it.
    pub interruption_info: u32,
    pub interruption_error: u32,
    /// I/O exits: port, access size in bytes and [`IO_IN`]/[`IO_STRING`]/[`IO_REP`].
    pub port: u16,
    pub io_size: u8,
    pub io_flags: u8,
    pub regs: Regs,
}

/// A segment register as VMX holds it. `access` is the VMCS access-rights format
/// (bit 16 = unusable).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segment { pub base: u64, pub limit: u32, pub access: u32, pub selector: u16, pub _pad: [u16; 3] }

/// The whole vCPU state `OP_VCPU_GET_REGS`/`SET_REGS` move. Segments are
/// ES, CS, SS, DS, FS, GS, LDTR, TR. `cr0`/`cr4` are what the guest sees.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub regs: Regs,
    pub cr0: u64, pub cr3: u64, pub cr4: u64, pub efer: u64,
    pub segments: [Segment; 8],
    pub gdtr_base: u64, pub idtr_base: u64, pub gdtr_limit: u32, pub idtr_limit: u32,
    pub interruptibility: u32, pub activity: u32,
}

impl State {
    /// A vCPU just after reset, in real mode at `cs:ip`.
    pub fn real_mode(cs: u16, ip: u16) -> State {
        let seg = |selector: u16, access: u32| Segment { base: (selector as u64) << 4, limit: 0xFFFF, access, selector, _pad: [0; 3] };
        let data = seg(0, 0x93);
        State {
            regs: Regs { rip: ip as u64, rflags: 2, ..Regs::default() },
            cr0: 0x10, // ET
            segments: [data, seg(cs, 0x9B), data, data, data, data, seg(0, 0x1_0082), seg(0, 0x8B)],
            gdtr_limit: 0xFFFF, idtr_limit: 0xFFFF,
            ..State::default()
        }
    }
}

/// The capability's data: which domain holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant { pub holder_pd: u32 }

/// Hardware encodings the kernel uses, kept here so the host can test them.
pub mod hw {
    pub const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
    /// An EPT entry pointing at the next table: read, write and execute below it.
    pub fn ept_table(frame: u64) -> u64 { (frame & ADDRESS) | 7 }
    /// A 4 KiB EPT leaf: access bits, write-back memory, ignore guest PAT.
    pub fn ept_leaf(frame: u64, access: u64) -> u64 { (frame & ADDRESS) | (access & 7) | (6 << 3) | (1 << 6) }
    /// Index into the EPT table at `level` (3 = PML4 .. 0 = page table).
    pub fn ept_index(gpa: u64, level: u32) -> usize { ((gpa >> (12 + 9 * level)) & 511) as usize }
    /// EPT pointer: write-back, 4-level walk.
    pub fn eptp(root: u64) -> u64 { (root & ADDRESS) | (3 << 3) | 6 }
    /// A VMX control value from its capability MSR (low half: bits that must be 1,
    /// high half: bits that may be 1): `required` must all be allowed, `optional`
    /// bits are taken where allowed.
    pub fn adjust(msr: u64, required: u32, optional: u32) -> Option<u32> {
        let (must, may) = (msr as u32, (msr >> 32) as u32);
        if required & !may != 0 { return None; }
        Some(must | required | (optional & may))
    }
    /// CR0/CR4 satisfy the VMX fixed bits.
    pub fn fixed_ok(value: u64, fixed0: u64, fixed1: u64) -> bool { value & fixed0 == fixed0 && value & !fixed1 == 0 }
    /// An I/O exit qualification: (port, size in bytes, IO_IN | IO_STRING | IO_REP).
    pub fn io(qualification: u64) -> (u16, u8, u8) {
        let flags = ((qualification >> 3) & 1) as u8 | (((qualification >> 4) & 1) as u8) << 1 | (((qualification >> 5) & 1) as u8) << 2;
        ((qualification >> 16) as u16, (qualification & 7) as u8 + 1, flags)
    }
    /// VMCS field of segment `i` (ES..TR): selector, limit, access rights, base.
    pub const fn segment_fields(i: u32) -> [u32; 4] { [0x800 + 2 * i, 0x4800 + 2 * i, 0x4814 + 2 * i, 0x6806 + 2 * i] }
    /// The base of a 64-bit system descriptor (TSS) from its two quadwords.
    pub fn descriptor_base(low: u64, high: u64) -> u64 {
        ((low >> 16) & 0xFF_FFFF) | (((low >> 56) & 0xFF) << 24) | ((high & 0xFFFF_FFFF) << 32)
    }
}

#[cfg(target_arch = "x86_64")]
fn call(op: u64, a: u64, b: u64, c: u64, d: u64) -> (u64, u64, u64) {
    let (status, r1, r2): (u64, u64, u64);
    unsafe { core::arch::asm!("syscall", inlateout("rax") SYS_VM => status, in("rdi") op,
        inlateout("rsi") a => r1, inlateout("rdx") b => r2, inlateout("r10") c => _, inlateout("r8") d => _,
        out("rcx") _, out("r11") _, options(nostack)); }
    (status, r1, r2)
}
#[cfg(not(target_arch = "x86_64"))]
fn call(_op: u64, _a: u64, _b: u64, _c: u64, _d: u64) -> (u64, u64, u64) { (UNAVAILABLE, 0, 0) }

fn ok(r: (u64, u64, u64)) -> Result<u64, u64> { if r.0 == OK { Ok(r.1) } else { Err(r.0) } }

/// (usable, reason code). Err when the caller holds no Machine capability.
pub fn info() -> Result<(bool, u64), u64> { let r = call(OP_INFO, 0, 0, 0, 0); if r.0 == OK { Ok((r.1 == 1, r.2)) } else { Err(r.0) } }
pub fn create() -> Result<u64, u64> { ok(call(OP_CREATE, 0, 0, 0, 0)) }
/// Lend `pages` heap pages starting at `address` to the VM at guest-physical `gpa`.
pub fn lend(vm: u64, address: u64, gpa: u64, pages: u32, access: u64) -> Result<(), u64> {
    ok(call(OP_LEND_MEMORY, vm, address, gpa, pages as u64 | access << 32)).map(|_| ())
}
pub fn vcpu_create(vm: u64) -> Result<u64, u64> { ok(call(OP_VCPU_CREATE, vm, 0, 0, 0)) }
/// Run until an exit the program must handle. Host interrupts and time-slice ends
/// return here too; this runs again after them. `load_regs`: the caller changed `exit.regs`.
pub fn run(vm: u64, vcpu: u64, exit: &mut Exit, load_regs: bool) -> Result<(), u64> {
    let mut flags = if load_regs { RUN_LOAD_REGS } else { 0 };
    loop {
        ok(call(OP_VCPU_RUN, vm, vcpu, exit as *mut Exit as u64, flags))?;
        if exit.reason != exit::EXTERNAL_INTERRUPT && exit.reason != exit::PREEMPTED { return Ok(()); }
        flags = 0;
    }
}
pub fn get_state(vm: u64, vcpu: u64, state: &mut State) -> Result<(), u64> {
    ok(call(OP_VCPU_GET_REGS, vm, vcpu, state as *mut State as u64, 0)).map(|_| ())
}
pub fn set_state(vm: u64, vcpu: u64, state: &State) -> Result<(), u64> {
    ok(call(OP_VCPU_SET_REGS, vm, vcpu, state as *const State as u64, 0)).map(|_| ())
}
pub fn inject(vm: u64, vcpu: u64, vector: u8, kind: u32, error: Option<u32>) -> Result<(), u64> {
    let event = vector as u64 | (kind as u64) << 8 | (error.is_some() as u64) << 11;
    ok(call(OP_INJECT, vm, vcpu, event, error.unwrap_or(0) as u64)).map(|_| ())
}
pub fn destroy(vm: u64) -> Result<(), u64> { ok(call(OP_DESTROY, vm, 0, 0, 0)).map(|_| ()) }

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn records_have_a_fixed_layout() {
        assert_eq!(size_of::<Regs>(), 144);
        assert_eq!(offset_of!(Regs, rdi), 40);
        assert_eq!(offset_of!(Regs, r15), 112);
        assert_eq!(offset_of!(Exit, qualification), 8);
        assert_eq!(offset_of!(Exit, port), 44);
        assert_eq!(offset_of!(Exit, regs), 48);
        assert_eq!(size_of::<Exit>(), 192);
        assert_eq!(size_of::<Segment>(), 24);
        assert_eq!(size_of::<State>(), 400);
    }

    #[test]
    fn ept_entries() {
        assert_eq!(hw::ept_table(0x1234_5000), 0x1234_5007);
        assert_eq!(hw::ept_leaf(0xABC_D000, READ | WRITE), 0xABC_D000 | 3 | 0x30 | 0x40);
        assert_eq!(hw::ept_leaf(0xFFF0_0000_0000_1FFF, READ), 0x1000 | 1 | 0x70, "stray bits dropped");
        assert_eq!(hw::eptp(0x5000), 0x501E);
        let gpa = (3u64 << 39) | (5 << 30) | (7 << 21) | (9 << 12) | 0x123;
        assert_eq!([3, 2, 1, 0].map(|l| hw::ept_index(gpa, l)), [3, 5, 7, 9]);
    }

    #[test]
    fn controls_and_fixed_bits() {
        let msr = (0x0000_00FFu64 << 32) | 0x16;
        assert_eq!(hw::adjust(msr, 0x01, 0x100), Some(0x17), "must-be-one kept, unsupported optional dropped");
        assert_eq!(hw::adjust(msr, 0x101, 0), None, "required bit not allowed");
        assert!(hw::fixed_ok(0x8000_0031, 0x8000_0021, 0xFFFF_FFFF));
        assert!(!hw::fixed_ok(0x8000_0011, 0x8000_0021, 0xFFFF_FFFF));
        assert!(!hw::fixed_ok(0x2000, 0, 0x1FFF));
    }

    #[test]
    fn io_qualification() {
        // out 0xE9, al: size 1, out, not string.
        assert_eq!(hw::io(0x00E9_0000), (0xE9, 1, 0));
        // rep insw from port 0x1F0: size 2, in, string, rep.
        assert_eq!(hw::io(0x01F0_0000 | 1 | 8 | 16 | 32), (0x1F0, 2, IO_IN | IO_STRING | IO_REP));
    }

    #[test]
    fn vmcs_segment_fields() {
        assert_eq!(hw::segment_fields(0), [0x800, 0x4800, 0x4814, 0x6806]);
        assert_eq!(hw::segment_fields(7), [0x80E, 0x480E, 0x4822, 0x6814]);
    }

    #[test]
    fn tss_descriptor_base() {
        // base 0xFFFF_8000_1234_5678 split the way the GDT stores it.
        let low = (0x5678u64 << 16) | (0x34u64 << 32) | (0x12u64 << 56) | 0x67;
        assert_eq!(hw::descriptor_base(low, 0xFFFF_8000), 0xFFFF_8000_1234_5678);
    }

    #[test]
    fn real_mode_reset_state() {
        let s = State::real_mode(0, 0x1000);
        assert_eq!(s.regs.rip, 0x1000);
        assert_eq!(s.regs.rflags, 2);
        assert_eq!(s.segments[1].access, 0x9B);
        assert_eq!(s.segments[6].access & 0x1_0000, 0x1_0000, "LDTR unusable");
        assert_eq!(State::real_mode(0xF000, 0).segments[1].base, 0xF0000);
    }
}
