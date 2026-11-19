//! Drag D5: the AMD SVM mechanism behind syscall 110 (`sex_pdx::vm`), beside VT-x in
//! `vmx.rs`, which keeps the VMs, loans and syscall and calls here on AMD machines.
//!
//! Kernel growth inside the Drag allowance (Andreas, 2026-11-14; docs/handoff/
//! DRAG_VIRTUAL_MACHINES_DESIGN_2026_11_14.md, phase D5): EFER.SVME, the VMCB, nested
//! paging and VMRUN only run in ring 0. Every exit goes back to the program as the same
//! VMX-shaped record (`sex_pdx::vm::svm::translate`), so drag, Crossdress and dragtest
//! run unchanged on Ryzen.
//!
//! Safety shape, as on Intel: SVM is entered only inside `OP_VCPU_RUN`, on the calling
//! core, and EFER.SVME is cleared again before the call returns. GIF is cleared around
//! the whole switch; the guest runs with the host's interrupts and NMIs intercepted
//! (V_INTR_MASKING, so the guest's IF and CR8 never mask the host), and they are taken
//! by the host right after STGI. CLGI, STGI, VMRUN, VMLOAD, VMSAVE, SKINIT, INVD and
//! XSETBV never run in a guest. The VMCB is rewritten from the vCPU's `State` on every
//! run (all clean bits off), so a vCPU may run on any core. Every guest uses ASID 1; a
//! core flushes it when it runs a different vCPU than last time, the vCPU last ran on
//! another core, or a nested table lost a mapping (`vmx::EPT_GENERATION`).
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::vm::{self as abi, svm, Reason};
use x86_64::registers::model_specific::Msr;

use crate::serial_println;

const CORES: usize = crate::scheduler::task_ownership::MAX_CORES as usize;
const EFER: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;
const EFER_SVME: u64 = 1 << 12;
const VM_CR: u32 = 0xC001_0114;
const VM_HSAVE_PA: u32 = 0xC001_0117;
const CR4_PKE: u64 = 1 << 22;

/// What this machine's SVM offers, probed once.
#[derive(Clone, Copy)]
pub struct Caps { nrips: bool, flush_by_asid: bool, nx: bool, iopm: u64, msrpm: u64 }

/// Per core: the VMCB, the host's VMSAVE area and the host save area (VM_HSAVE_PA).
static REGIONS: [AtomicU64; CORES * 3] = [const { AtomicU64::new(0) }; CORES * 3];
/// Per core: the vCPU that ran last and the nested-table generation it flushed at.
static LAST: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];
static SEEN: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];
/// The I/O (12 KiB) and MSR (8 KiB) permission maps, every bit set: every port and
/// MSR access exits. Shared by every guest, read only by the CPU.
static MAPS: AtomicU64 = AtomicU64::new(0);

fn cpuid(leaf: u32) -> core::arch::x86_64::CpuidResult { core::arch::x86_64::__cpuid_count(leaf, 0) }
fn rdmsr(n: u32) -> u64 { unsafe { Msr::new(n).read() } }
fn hhdm(frame: u64) -> u64 { frame + crate::HHDM_REQUEST.response().expect("SVM_HHDM").offset }

/// SVM is on this CPU (CPUID 8000_0001 ECX.SVM): the VMX path is not.
pub fn present() -> bool { cpuid(0x8000_0000).eax >= 0x8000_000A && cpuid(0x8000_0001).ecx & (1 << 2) != 0 }

/// Firmware lock, nested paging and ASIDs; then the permission maps. Logs
/// `[drag] svm=...` the first time (`vmx::caps`).
pub fn probe() -> Result<Caps, Reason> {
    if rdmsr(VM_CR) & (1 << 4) != 0 { return Err(Reason::SvmDisabled); }
    let leaf = cpuid(0x8000_000A);
    if leaf.edx & 1 == 0 { return Err(Reason::NoNpt); }
    if leaf.ebx < 2 { return Err(Reason::Controls); }
    let maps = match MAPS.load(Ordering::Acquire) {
        0 => {
            // Order 3: 32 KiB, the I/O map in the first 12 KiB, the MSR map at 16 KiB.
            let base = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(3).ok_or(Reason::Controls)?;
            unsafe { core::ptr::write_bytes(hhdm(base) as *mut u8, 0xFF, 8 * 4096) };
            MAPS.store(base, Ordering::Release);
            base
        }
        m => m,
    };
    Ok(Caps { nrips: leaf.edx & (1 << 3) != 0, flush_by_asid: leaf.edx & (1 << 6) != 0, nx: rdmsr(EFER) & EFER_NXE != 0,
        iopm: maps, msrpm: maps + 4 * 4096 })
}

pub fn describe(c: &Caps) {
    serial_println!("[drag] svm=available asids={} npt=1 nrips={} flush_by_asid={} nx={} decode_assists={}",
        cpuid(0x8000_000A).ebx, c.nrips as u8, c.flush_by_asid as u8, c.nx as u8, (cpuid(0x8000_000A).edx >> 7) & 1);
}

/// The nested-table entries `vmx::map` writes on this machine.
pub fn table(frame: u64) -> u64 { svm::npt_table(frame) }
pub fn leaf(c: &Caps, frame: u64, access: u64) -> u64 { svm::npt_leaf(frame, access, c.nx) }

fn region(i: usize) -> Option<u64> {
    let held = REGIONS[i].load(Ordering::Acquire);
    if held != 0 { return Some(held); }
    let frame = crate::program_frames::take()?;
    REGIONS[i].store(frame, Ordering::Release);
    Some(frame)
}

// VMCB offsets (APM volume 2, appendix B).
const INTERCEPT_3: usize = 0x0C; const INTERCEPT_4: usize = 0x10;
const IOPM: usize = 0x40; const MSRPM: usize = 0x48; const ASID: usize = 0x58; const TLB_CONTROL: usize = 0x5C;
const V_INTR: usize = 0x60; const SHADOW: usize = 0x68; const EXIT_CODE: usize = 0x70; const EXIT_INFO1: usize = 0x78;
const EXIT_INFO2: usize = 0x80; const EXIT_INT_INFO: usize = 0x88; const NP_ENABLE: usize = 0x90; const EVENT_INJ: usize = 0xA8;
const N_CR3: usize = 0xB0; const NEXT_RIP: usize = 0xC8;
/// ES, CS, SS, DS, FS, GS, LDTR, TR in `State` order; GDTR and IDTR between them.
const SEGMENTS: [usize; 8] = [0x400, 0x410, 0x420, 0x430, 0x440, 0x450, 0x470, 0x490];
const GDTR: usize = 0x460; const IDTR: usize = 0x480; const CPL: usize = 0x4CB; const GUEST_EFER: usize = 0x4D0;
const CR4: usize = 0x548; const CR3: usize = 0x550; const CR0: usize = 0x558; const DR7: usize = 0x560; const DR6: usize = 0x568;
const RFLAGS: usize = 0x570; const RIP: usize = 0x578; const RSP: usize = 0x5D8; const RAX: usize = 0x5F8;
const KERNEL_GS: usize = 0x620; const CR2: usize = 0x640; const PAT: usize = 0x668;

struct Vmcb(*mut u8);
impl Vmcb {
    unsafe fn w<T>(&self, at: usize, v: T) { (self.0.add(at) as *mut T).write_volatile(v) }
    unsafe fn r<T: Copy>(&self, at: usize) -> T { (self.0.add(at) as *const T).read_volatile() }
}

/// One VMRUN and #VMEXIT on this core. `key` names the vCPU (for the TLB), `extra` is
/// its CR2 and KERNEL_GS_BASE, which `State` does not carry, and the core it last ran on
/// plus one (0 = never ran). Restores PKRU (guest
/// WRPKRU) and EFER; VMRUN and VMLOAD restore the rest of the host.
pub unsafe fn enter(c: &Caps, core: usize, key: u64, st: &mut abi::State, extra: &mut [u64; 3], ncr3: u64, event: u32, error: u32,
    exit: &mut abi::Exit) -> Result<(u32, u32), (u64, u64)> {
    use x86_64::registers::control::Cr4;
    let (Some(vmcb), Some(host), Some(hsave)) = (region(core * 3), region(core * 3 + 1), region(core * 3 + 2)) else { return Err((abi::EXHAUSTED, 0)) };
    let v = Vmcb(hhdm(vmcb) as *mut u8);
    core::ptr::write_bytes(v.0, 0, 4096);
    let generation = crate::vmx::EPT_GENERATION.load(Ordering::Acquire);
    // Flush when this core ran another vCPU since, a nested table lost a mapping, or the
    // vCPU ran elsewhere last: its INVLPGs there never reached this core's TLB.
    let moved = extra[2] != core as u64 + 1;
    extra[2] = core as u64 + 1;
    let flush = (SEEN[core].swap(generation, Ordering::AcqRel) != generation) | (LAST[core].swap(key, Ordering::AcqRel) != key) | moved;
    load(c, &v, st, extra, ncr3, event, error, flush);
    let cr4 = Cr4::read_raw();
    let pkru = if cr4 & CR4_PKE != 0 { crate::pku::rdpkru() } else { 0 };
    let efer = rdmsr(EFER);
    Msr::new(EFER).write(efer | EFER_SVME);
    Msr::new(VM_HSAVE_PA).write(hsave);
    sexos_svm_enter(&mut st.regs, vmcb, host);
    Msr::new(EFER).write(efer);
    if cr4 & CR4_PKE != 0 { crate::pku::wrpkru(pkru); }
    Ok(save(c, &v, st, extra, exit))
}

unsafe fn load(c: &Caps, v: &Vmcb, st: &abi::State, extra: &[u64; 3], ncr3: u64, event: u32, error: u32, flush: bool) {
    v.w(INTERCEPT_3, svm::INTERCEPT_3); v.w(INTERCEPT_4, svm::INTERCEPT_4);
    v.w(IOPM, c.iopm); v.w(MSRPM, c.msrpm);
    v.w(ASID, 1u32);
    v.w(TLB_CONTROL, if !flush { 0u8 } else if c.flush_by_asid { 3 } else { 1 });
    v.w(V_INTR, 1u64 << 24); // V_INTR_MASKING
    v.w(SHADOW, (st.interruptibility & 3 != 0) as u64);
    v.w(NP_ENABLE, 1u64);
    v.w(EVENT_INJ, if event & (1 << 31) != 0 { event as u64 | (error as u64) << 32 } else { 0 });
    v.w(N_CR3, ncr3);
    for (s, &at) in st.segments.iter().zip(SEGMENTS.iter()) {
        v.w(at, s.selector); v.w(at + 2, svm::attrib(s.access)); v.w(at + 4, s.limit); v.w(at + 8, s.base);
    }
    v.w(GDTR + 4, st.gdtr_limit); v.w(GDTR + 8, st.gdtr_base);
    v.w(IDTR + 4, st.idtr_limit); v.w(IDTR + 8, st.idtr_base);
    v.w(CPL, svm::cpl(st.cr0, st.regs.rflags, svm::attrib(st.segments[2].access)));
    v.w(GUEST_EFER, st.efer | EFER_SVME); // VMRUN requires it; the guest never sees it
    v.w(CR0, st.cr0); v.w(CR3, st.guest_cr3); v.w(CR4, st.cr4);
    v.w(DR7, 0x400u64); v.w(DR6, 0xFFFF_0FF0u64);
    v.w(RFLAGS, (st.regs.rflags & 0x003F_7FD5) | 2);
    v.w(RIP, st.regs.rip); v.w(RSP, st.regs.rsp); v.w(RAX, st.regs.rax);
    v.w(KERNEL_GS, extra[1]); v.w(CR2, extra[0]);
    v.w(PAT, 0x0007_0406_0007_0406u64);
}

unsafe fn save(c: &Caps, v: &Vmcb, st: &mut abi::State, extra: &mut [u64; 3], exit: &mut abi::Exit) -> (u32, u32) {
    st.regs.rax = v.r(RAX); st.regs.rsp = v.r(RSP); st.regs.rip = v.r(RIP); st.regs.rflags = v.r(RFLAGS);
    st.cr0 = v.r(CR0); st.guest_cr3 = v.r(CR3); st.cr4 = v.r(CR4);
    st.efer = v.r::<u64>(GUEST_EFER) & !EFER_SVME;
    for (s, &at) in st.segments.iter_mut().zip(SEGMENTS.iter()) {
        s.selector = v.r(at); s.access = svm::access(v.r(at + 2)); s.limit = v.r(at + 4); s.base = v.r(at + 8);
    }
    st.gdtr_limit = v.r(GDTR + 4); st.gdtr_base = v.r(GDTR + 8);
    st.idtr_limit = v.r(IDTR + 4); st.idtr_base = v.r(IDTR + 8);
    st.interruptibility = (v.r::<u64>(SHADOW) & 1) as u32;
    st.activity = 0;
    extra[0] = v.r(CR2); extra[1] = v.r(KERNEL_GS);
    let code: u64 = v.r(EXIT_CODE);
    let t = svm::translate(code, v.r(EXIT_INFO1), v.r(EXIT_INFO2), st.regs.rip, c.nrips.then(|| v.r(NEXT_RIP)));
    *exit = abi::Exit { reason: t.reason, instruction_len: t.instruction_len, qualification: t.qualification,
        guest_physical: t.guest_physical, interruption_info: t.interruption_info, interruption_error: t.interruption_error,
        port: t.port, io_size: t.io_size, io_flags: t.io_flags, regs: st.regs, ..abi::Exit::default() };
    let pending: u64 = v.r(EXIT_INT_INFO);
    if pending & (1 << 31) == 0 || code == svm::INVALID { return (0, 0); }
    (pending as u32, (pending >> 32) as u32)
}

extern "C" { fn sexos_svm_enter(regs: *mut abi::Regs, vmcb: u64, host: u64); }
// rdi = the guest's registers, rsi = the VMCB, rdx = the host's VMSAVE area (physical).
// Called with interrupts off. CLGI holds every interrupt and NMI across the switch;
// STI lets the guest be interrupted (V_INTR_MASKING: the host's IF decides), and after
// #VMEXIT (GIF still 0) CLI then STGI leaves them pending for the host. RAX, RSP, RIP and
// RFLAGS travel in the VMCB; the other general registers are swapped here.
core::arch::global_asm!(r#"
.global sexos_svm_enter
sexos_svm_enter:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    push rdx
    push rsi
    push rdi
    clgi
    mov rax, rdx
    vmsave rax
    mov rax, rsi
    vmload rax
    mov rbx, [rdi + 8]
    mov rcx, [rdi + 16]
    mov rdx, [rdi + 24]
    mov rsi, [rdi + 32]
    mov rbp, [rdi + 48]
    mov r8, [rdi + 56]
    mov r9, [rdi + 64]
    mov r10, [rdi + 72]
    mov r11, [rdi + 80]
    mov r12, [rdi + 88]
    mov r13, [rdi + 96]
    mov r14, [rdi + 104]
    mov r15, [rdi + 112]
    mov rdi, [rdi + 40]
    sti
    vmrun rax
    push rdi
    mov rdi, [rsp + 8]
    mov [rdi + 8], rbx
    mov [rdi + 16], rcx
    mov [rdi + 24], rdx
    mov [rdi + 32], rsi
    mov [rdi + 48], rbp
    mov [rdi + 56], r8
    mov [rdi + 64], r9
    mov [rdi + 72], r10
    mov [rdi + 80], r11
    mov [rdi + 88], r12
    mov [rdi + 96], r13
    mov [rdi + 104], r14
    mov [rdi + 112], r15
    pop rax
    mov [rdi + 40], rax
    mov rax, [rsp + 8]
    vmsave rax
    mov rax, [rsp + 16]
    vmload rax
    cli
    stgi
    add rsp, 24
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
"#);
