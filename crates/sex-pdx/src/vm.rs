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
//! Intel VT-x (D1) and AMD SVM (D5). Either is entered inside `OP_VCPU_RUN` on the
//! calling core and left before the call returns, so a program that never calls this
//! leaves VMX and SVM untouched. On AMD the kernel turns every SVM exit into the same
//! [`Exit`] record (Intel exit reasons and qualifications, see [`svm::translate`]), so a
//! VMM is written once. Without usable VMX or SVM (firmware lock, no nested paging)
//! every op but [`OP_INFO`] answers [`UNAVAILABLE`] and `OP_INFO` says why.
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
pub enum Reason { Usable = 0, NoVmx = 1, FirmwareLocked = 2, Controls = 3, Ept = 4, Basic = 5, SvmDisabled = 6, NoNpt = 7 }
impl Reason {
    pub fn name(self) -> &'static str {
        match self { Reason::Usable => "usable", Reason::NoVmx => "no_vmx_cpuid", Reason::FirmwareLocked => "feature_control_locked",
            Reason::Controls => "missing_controls", Reason::Ept => "missing_ept", Reason::Basic => "vmx_basic",
            Reason::SvmDisabled => "svm_disabled_by_firmware", Reason::NoNpt => "missing_npt" }
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
    pub const INVD: u32 = 13;
    pub const RDPMC: u32 = 15;
    pub const VMCALL: u32 = 18;
    /// The guest ran a virtualization instruction (VMLAUNCH on Intel; VMRUN, VMLOAD,
    /// VMSAVE, STGI, CLGI, SKINIT or INVLPGA on AMD). A VMM usually injects #UD.
    pub const VMX_INSTRUCTION: u32 = 20;
    pub const CR_ACCESS: u32 = 28;
    pub const IO: u32 = 30;
    pub const RDMSR: u32 = 31;
    pub const WRMSR: u32 = 32;
    pub const EPT_VIOLATION: u32 = 48;
    pub const EPT_MISCONFIG: u32 = 49;
    pub const MWAIT: u32 = 36;
    pub const MONITOR: u32 = 39;
    pub const WBINVD: u32 = 54;
    /// The vCPU's time slice ended (fairness). Run again.
    pub const PREEMPTED: u32 = 52;
    pub const XSETBV: u32 = 55;
    /// Bit 31 of `Exit::reason`: the entry failed on guest state (reason 33/34).
    pub const ENTRY_FAILURE: u32 = 1 << 31;
    /// Entry failed on invalid guest state (with [`ENTRY_FAILURE`]).
    pub const INVALID_GUEST_STATE: u32 = 33;
    /// Bit 30: an AMD exit code with no Intel counterpart; the low bits are the code.
    pub const SVM_OTHER: u32 = 1 << 30;
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
    pub cr0: u64, pub guest_cr3: u64, pub cr4: u64, pub efer: u64,
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

/// AMD SVM encodings the kernel uses (AMD64 APM volume 2, chapter 15 and appendix B),
/// kept here so the host can test them. The kernel turns every SVM exit into the
/// VMX-shaped [`Exit`] with [`translate`], so VMMs see one format on both vendors.
pub mod svm {
    use super::{exit, IO_IN, IO_STRING, IO_REP, EXEC, WRITE};

    /// Nested page table entries are ordinary x86-64 entries: present, writable, user
    /// (the guest's accesses count as user accesses in the nested walk).
    pub fn npt_table(frame: u64) -> u64 { (frame & super::hw::ADDRESS) | 7 }
    /// A 4 KiB nested leaf. Write-back (PAT index 0). Not executable unless `EXEC`,
    /// when the host runs with EFER.NXE (`nx`); without it every page is executable.
    pub fn npt_leaf(frame: u64, access: u64, nx: bool) -> u64 {
        let w = if access & WRITE != 0 { 2 } else { 0 };
        let no_exec = if nx && access & EXEC == 0 { 1 << 63 } else { 0 };
        (frame & super::hw::ADDRESS) | 1 | w | 4 | no_exec
    }

    /// VMX access rights (bits 7:0 type/S/DPL/P, 15:12 AVL/L/DB/G, bit 16 unusable)
    /// to the VMCB's packed 12-bit attribute. An unusable segment is null (P = 0).
    pub fn attrib(access: u32) -> u16 {
        if access & (1 << 16) != 0 { return 0; }
        ((access & 0xFF) | ((access >> 4) & 0xF00)) as u16
    }
    /// The VMCB attribute back to VMX access rights; not present means unusable.
    pub fn access(attrib: u16) -> u32 {
        let a = attrib as u32;
        (a & 0xFF) | ((a & 0xF00) << 4) | if a & 0x80 == 0 { 1 << 16 } else { 0 }
    }

    // SVM exit codes.
    pub const EXCEPTION_BASE: u64 = 0x40;
    pub const INTR: u64 = 0x60;
    pub const NMI: u64 = 0x61;
    pub const SMI: u64 = 0x62;
    pub const INIT: u64 = 0x63;
    pub const RDPMC: u64 = 0x6F;
    pub const CPUID: u64 = 0x72;
    pub const INVD: u64 = 0x76;
    pub const HLT: u64 = 0x78;
    pub const INVLPGA: u64 = 0x7A;
    pub const IOIO: u64 = 0x7B;
    pub const MSR: u64 = 0x7C;
    pub const SHUTDOWN: u64 = 0x7F;
    pub const VMRUN: u64 = 0x80;
    pub const VMMCALL: u64 = 0x81;
    pub const VMLOAD: u64 = 0x82;
    pub const VMSAVE: u64 = 0x83;
    pub const STGI: u64 = 0x84;
    pub const CLGI: u64 = 0x85;
    pub const SKINIT: u64 = 0x86;
    pub const WBINVD: u64 = 0x89;
    pub const MONITOR: u64 = 0x8A;
    pub const MWAIT: u64 = 0x8B;
    pub const MWAIT_CONDITIONAL: u64 = 0x8C;
    pub const XSETBV: u64 = 0x8D;
    pub const NPF: u64 = 0x400;
    pub const INVALID: u64 = u64::MAX;

    /// Intercept vector 3 (VMCB 0x0C): what exits. External interrupts, NMIs and INIT
    /// stay the host's; CPUID, HLT, every port and MSR go to the VMM (like Intel's
    /// unconditional I/O exiting without MSR bitmaps); INVD and INVLPGA never run in a guest.
    pub const INTERCEPT_3: u32 = 1 | (1 << 1) | (1 << 3) | (1 << 15) | (1 << 18) | (1 << 22) | (1 << 24)
        | (1 << 26) | (1 << 27) | (1 << 28) | (1u32 << 31);
    /// Intercept vector 4 (VMCB 0x10): VMRUN (required), VMMCALL, VMLOAD, VMSAVE, STGI,
    /// CLGI (a guest must never hold the host's interrupts), SKINIT, WBINVD, MONITOR,
    /// MWAIT and XSETBV (XCR0 is not swapped by VMRUN).
    pub const INTERCEPT_4: u32 = 0x3F | (1 << 6) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12) | (1 << 13);

    /// One SVM exit in the VMX-shaped form of [`super::Exit`].
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Translated {
        pub reason: u32, pub instruction_len: u32, pub qualification: u64, pub guest_physical: u64,
        pub interruption_info: u32, pub interruption_error: u32, pub port: u16, pub io_size: u8, pub io_flags: u8,
    }

    /// Instruction length when the CPU gives no next RIP (no NRIP save): the exits that
    /// are skipped have fixed encodings (prefixes aside).
    fn fixed_len(code: u64) -> u32 {
        match code {
            HLT => 1,
            CPUID | MSR | INVD | WBINVD | RDPMC => 2,
            VMMCALL | XSETBV | MONITOR | MWAIT | MWAIT_CONDITIONAL | VMRUN | VMLOAD | VMSAVE | STGI | CLGI | SKINIT | INVLPGA => 3,
            _ => 0,
        }
    }

    /// Turns an SVM exit (EXITCODE, EXITINFO1, EXITINFO2, the guest RIP and the next
    /// RIP when the CPU saves it) into Intel's exit reason and qualification.
    pub fn translate(code: u64, info1: u64, info2: u64, rip: u64, next_rip: Option<u64>) -> Translated {
        let mut t = Translated::default();
        let len = |next: u64| { let l = next.wrapping_sub(rip); if l >= 1 && l <= 15 { l as u32 } else { 0 } };
        t.instruction_len = next_rip.map(len).filter(|&l| l != 0).unwrap_or_else(|| fixed_len(code));
        t.reason = match code {
            c if (EXCEPTION_BASE..EXCEPTION_BASE + 32).contains(&c) => {
                let vector = (c - EXCEPTION_BASE) as u32;
                let has_error = matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30);
                t.interruption_info = vector | (3 << 8) | (has_error as u32) << 11 | 1 << 31;
                t.interruption_error = if has_error { info1 as u32 } else { 0 };
                if vector == 14 { t.qualification = info2; }
                t.instruction_len = 0;
                exit::EXCEPTION_NMI
            }
            INTR | NMI | SMI | INIT => { t.instruction_len = 0; exit::EXTERNAL_INTERRUPT }
            SHUTDOWN => { t.instruction_len = 0; exit::TRIPLE_FAULT }
            CPUID => exit::CPUID,
            HLT => exit::HLT,
            INVD => exit::INVD,
            RDPMC => exit::RDPMC,
            VMMCALL => exit::VMCALL,
            VMRUN | VMLOAD | VMSAVE | STGI | CLGI | SKINIT | INVLPGA => exit::VMX_INSTRUCTION,
            WBINVD => exit::WBINVD,
            MONITOR => exit::MONITOR,
            MWAIT | MWAIT_CONDITIONAL => exit::MWAIT,
            XSETBV => exit::XSETBV,
            MSR => if info1 & 1 != 0 { exit::WRMSR } else { exit::RDMSR },
            IOIO => {
                // EXITINFO2 holds the next RIP whether or not NRIP save exists.
                if let l @ 1.. = len(info2) { t.instruction_len = l; }
                let size: u8 = if info1 & (1 << 4) != 0 { 1 } else if info1 & (1 << 5) != 0 { 2 } else { 4 };
                let input = info1 & 1 != 0;
                let (string, rep) = (info1 & (1 << 2) != 0, info1 & (1 << 3) != 0);
                t.port = (info1 >> 16) as u16;
                t.io_size = size;
                t.io_flags = (input as u8 * IO_IN) | (string as u8 * IO_STRING) | (rep as u8 * IO_REP);
                t.qualification = (size as u64 - 1) | (input as u64) << 3 | (string as u64) << 4 | (rep as u64) << 5 | (t.port as u64) << 16;
                exit::IO
            }
            NPF => {
                // Error code: P, W, U, RSV, fetch; Intel: read/write/fetch, then what the
                // entry allowed (a present entry that faulted forbade this access).
                let (present, write, fetch) = (info1 & 1 != 0, info1 & 2 != 0, info1 & 16 != 0);
                let read = !write && !fetch;
                let allowed = if present { (1 << 3) | ((!write as u64) << 4) | ((!fetch as u64) << 5) } else { 0 };
                t.qualification = read as u64 | (write as u64) << 1 | (fetch as u64) << 2 | allowed | (info1 >> 32 & 1) << 8;
                t.guest_physical = info2;
                t.instruction_len = 0;
                exit::EPT_VIOLATION
            }
            INVALID => { t.instruction_len = 0; exit::ENTRY_FAILURE | exit::INVALID_GUEST_STATE }
            other => { t.instruction_len = 0; exit::SVM_OTHER | (other as u32 & 0xFFFF) }
        };
        t
    }

    /// The CPL the VMCB must carry: 0 in real mode, 3 in virtual-8086 mode, else SS.DPL.
    pub fn cpl(cr0: u64, rflags: u64, ss_attrib: u16) -> u8 {
        if cr0 & 1 == 0 { 0 } else if rflags & (1 << 17) != 0 { 3 } else { ((ss_attrib >> 5) & 3) as u8 }
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

    #[test]
    fn svm_nested_entries() {
        assert_eq!(svm::npt_table(0x1234_5FFF), 0x1234_5007);
        assert_eq!(svm::npt_leaf(0xABC_D000, READ | WRITE, true), 0xABC_D000 | 7 | 1 << 63, "no exec without EXEC");
        assert_eq!(svm::npt_leaf(0xABC_D000, READ | EXEC, true), 0xABC_D000 | 5, "read-only, executable");
        assert_eq!(svm::npt_leaf(0xABC_D000, READ, false), 0xABC_D000 | 5, "no NX bit when the host lacks NXE");
    }

    #[test]
    fn svm_segment_attributes() {
        // 64-bit code: type B, S, P, L.
        assert_eq!(svm::attrib(0xA09B), 0xA9B);
        assert_eq!(svm::access(0xA9B), 0xA09B);
        // Real-mode data and the unusable LDTR.
        assert_eq!(svm::attrib(0x93), 0x93);
        assert_eq!(svm::attrib(0x1_0082), 0);
        assert_eq!(svm::access(0) & 0x1_0000, 0x1_0000, "not present comes back unusable");
        for a in [0x9Bu32, 0x93, 0x8B, 0xC093, 0xA09B, 0x40F3] { assert_eq!(svm::access(svm::attrib(a)), a); }
    }

    #[test]
    fn svm_io_exit_matches_vmx() {
        // out 0xE9, al at rip 0x7C00 (2 bytes): SZ8, port 0xE9, next rip in EXITINFO2.
        let t = svm::translate(svm::IOIO, 0x00E9_0010, 0x7C02, 0x7C00, None);
        assert_eq!((t.reason, t.port, t.io_size, t.io_flags, t.instruction_len), (exit::IO, 0xE9, 1, 0, 2));
        assert_eq!(hw::io(t.qualification), (0xE9, 1, 0), "the qualification decodes as Intel's");
        // rep insw from 0x1F0.
        let t = svm::translate(svm::IOIO, 0x01F0_0000 | 1 | 4 | 8 | 32, 0x1002, 0x1000, Some(0x1002));
        assert_eq!((t.port, t.io_size, t.io_flags), (0x1F0, 2, IO_IN | IO_STRING | IO_REP));
        assert_eq!(hw::io(t.qualification), (0x1F0, 2, IO_IN | IO_STRING | IO_REP));
        // out dx, eax.
        assert_eq!(svm::translate(svm::IOIO, 0x0CF8_0040, 1, 0, None).io_size, 4);
    }

    #[test]
    fn svm_exits_and_lengths() {
        let t = |c, i1| svm::translate(c, i1, 0, 0x100, None);
        assert_eq!((t(svm::CPUID, 0).reason, t(svm::CPUID, 0).instruction_len), (exit::CPUID, 2));
        assert_eq!((t(svm::HLT, 0).reason, t(svm::HLT, 0).instruction_len), (exit::HLT, 1));
        assert_eq!(t(svm::MSR, 0).reason, exit::RDMSR);
        assert_eq!(t(svm::MSR, 1).reason, exit::WRMSR);
        assert_eq!((t(svm::VMMCALL, 0).reason, t(svm::VMMCALL, 0).instruction_len), (exit::VMCALL, 3));
        assert_eq!(t(svm::XSETBV, 0).reason, exit::XSETBV);
        assert_eq!(t(svm::CLGI, 0).reason, exit::VMX_INSTRUCTION);
        assert_eq!(t(svm::INTR, 0).reason, exit::EXTERNAL_INTERRUPT);
        assert_eq!(t(svm::NMI, 0).reason, exit::EXTERNAL_INTERRUPT, "the host takes its NMI after STGI");
        assert_eq!(t(svm::SHUTDOWN, 0).reason, exit::TRIPLE_FAULT);
        assert_eq!(t(svm::INVALID, 0).reason, exit::ENTRY_FAILURE | exit::INVALID_GUEST_STATE);
        assert_eq!(t(0x6E, 0).reason, exit::SVM_OTHER | 0x6E);
        // NRIP save wins over the fixed table (a prefixed CPUID is 3 bytes).
        assert_eq!(svm::translate(svm::CPUID, 0, 0, 0x100, Some(0x103)).instruction_len, 3);
        assert_eq!(svm::translate(svm::CPUID, 0, 0, 0x100, Some(0x100)).instruction_len, 2, "a nonsense next rip is ignored");
    }

    #[test]
    fn svm_nested_page_fault() {
        // A write to a page lent read-only: present, write, final translation.
        let t = svm::translate(svm::NPF, 1 | 2 | 4 | 1 << 32, 0xD000_0010, 0, None);
        assert_eq!((t.reason, t.guest_physical, t.instruction_len), (exit::EPT_VIOLATION, 0xD000_0010, 0));
        assert_eq!(t.qualification, 2 | 8 | 32 | 256);
        // A read of an unlent MMIO page: not present.
        assert_eq!(svm::translate(svm::NPF, 4, 0xD000_0000, 0, None).qualification, 1);
        // A page fault the VMM asked to see: vector 14 with its error code and address.
        let t = svm::translate(svm::EXCEPTION_BASE + 14, 2, 0xDEAD_B000, 0, None);
        assert_eq!((t.reason, t.interruption_info, t.interruption_error, t.qualification), (exit::EXCEPTION_NMI, 14 | 0x300 | 0x800 | 1 << 31, 2, 0xDEAD_B000));
    }

    #[test]
    fn svm_cpl() {
        assert_eq!(svm::cpl(0x10, 2, 0x93), 0, "real mode");
        assert_eq!(svm::cpl(0x8000_0011, 2, 0xF3), 3, "SS.DPL 3");
        assert_eq!(svm::cpl(0x11, 2 | 1 << 17, 0xF3), 3, "virtual-8086");
        assert_eq!(svm::cpl(0x8000_0011, 2, 0xC93), 0);
    }
}
