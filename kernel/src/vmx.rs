//! Drag D1: the Intel VT-x mechanism behind syscall 110 (`sex_pdx::vm`).
//!
//! Kernel growth, approved by Andreas on 2026-11-14 (docs/handoff/DRAG_VIRTUAL_MACHINES_
//! DESIGN_2026_11_14.md): VMX instructions, the VMCS, EPT and VM entry/exit only run in
//! ring 0. Everything a VMM decides (devices, I/O, CPUID, MSRs, HLT) goes back to the
//! program as an exit record.
//!
//! Safety shape: VMX is entered only inside `OP_VCPU_RUN`, on the calling core, with
//! interrupts off, and left (VMCLEAR, VMXOFF, CR4.VMXE cleared) before the call returns.
//! A machine whose programs never call SYS_VM never runs a VMX instruction, and no core
//! stays in VMX operation between calls, so INIT/SIPI and the stop screen behave as before.
//! The vCPU lives in memory as a `sex_pdx::vm::State`; every run writes the whole VMCS
//! from it, so a vCPU may run on any core. A guest reaches only frames of its program's
//! own heap (checked by `app_resources::heap_frames`), and the VM ends before the run's
//! frames go back (`revoke`, called by `app_resources::retire`).
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::vm::{self as abi, hw, Reason};
use spin::Mutex;
use x86_64::registers::model_specific::Msr;

use crate::capability::CapabilityData;
use crate::serial_println;

const MAX_VMS: usize = 4; // per program run
const MAX_VCPUS: usize = 8; // per VM
/// Highest guest-physical address a loan may reach (512 GiB).
const GPA_LIMIT: u64 = 1 << 39;
/// VMX preemption timer slice in TSC cycles (a few ms at most), so a vCPU in a tight
/// loop returns to ring 3 and the scheduler even without a host interrupt.
const SLICE_TSC: u64 = 4_000_000;
const CORES: usize = crate::scheduler::task_ownership::MAX_CORES as usize;

struct Vcpu { state: abi::State, event: u32, event_error: u32, running: bool }
struct Vm { id: u64, pd: u32, root: u64, tables: Vec<u64>, loans: Vec<(u64, u64)>, vcpus: Vec<Vcpu> }
struct Machines { next: u64, vms: Vec<Vm> }
static VMS: Mutex<Machines> = Mutex::new(Machines { next: 1, vms: Vec::new() });

/// What this machine's VMX offers, read once on the first SYS_VM call.
#[derive(Clone, Copy)]
struct Caps { revision: u32, pin: u32, proc: u32, proc2: u32, exit: u32, entry: u32, entry_may: u32,
    cr0: (u64, u64), cr4: (u64, u64), timer_shift: Option<u8> }
static CAPS: Mutex<Option<Result<Caps, Reason>>> = Mutex::new(None);

/// Per core: the VMXON region and the VMCS every run on that core uses (taken on first use).
static REGIONS: [AtomicU64; CORES * 2] = [const { AtomicU64::new(0) }; CORES * 2];
/// Bumped whenever an EPT loses a mapping or is freed. A core whose last INVEPT is older
/// flushes every EPT translation before its next entry, so no stale guest-physical
/// translation outlives a destroyed VM on any core.
static EPT_GENERATION: AtomicU64 = AtomicU64::new(1);
static SEEN: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];

// VMCS fields (Intel SDM appendix B).
const PIN: u32 = 0x4000; const PROC: u32 = 0x4002; const PROC2: u32 = 0x401E;
const EXIT_CTL: u32 = 0x400C; const ENTRY_CTL: u32 = 0x4012; const ENTRY_EVENT: u32 = 0x4016; const ENTRY_ERROR: u32 = 0x4018;
const EPT_POINTER: u32 = 0x201A; const LINK: u32 = 0x2800; const GUEST_DEBUGCTL: u32 = 0x2802; const GUEST_EFER: u32 = 0x2806;
const HOST_EFER: u32 = 0x2C02; const GUEST_PHYSICAL: u32 = 0x2400;
const INSTRUCTION_ERROR: u32 = 0x4400; const REASON: u32 = 0x4402; const EXIT_EVENT: u32 = 0x4404; const EXIT_EVENT_ERROR: u32 = 0x4406;
const IDT_VECTORING: u32 = 0x4408; const IDT_VECTORING_ERROR: u32 = 0x440A; const INSTRUCTION_LEN: u32 = 0x440C; const INSTRUCTION_INFO: u32 = 0x440E;
const GDTR_LIMIT: u32 = 0x4810; const IDTR_LIMIT: u32 = 0x4812; const INTERRUPTIBILITY: u32 = 0x4824; const ACTIVITY: u32 = 0x4826;
const TIMER: u32 = 0x482E; const HOST_SYSENTER_CS: u32 = 0x4C00;
const CR0_MASK: u32 = 0x6000; const CR4_MASK: u32 = 0x6002; const CR0_SHADOW: u32 = 0x6004; const CR4_SHADOW: u32 = 0x6006;
const QUALIFICATION: u32 = 0x6400; const GUEST_LINEAR: u32 = 0x640A;
const GUEST_CR0: u32 = 0x6800; const GUEST_CR3: u32 = 0x6802; const GUEST_CR4: u32 = 0x6804; const GDTR_BASE: u32 = 0x6816; const IDTR_BASE: u32 = 0x6818;
const GUEST_DR7: u32 = 0x681A; const GUEST_RSP: u32 = 0x681C; const GUEST_RIP: u32 = 0x681E; const GUEST_RFLAGS: u32 = 0x6820;
const HOST_CR0: u32 = 0x6C00; const HOST_CR3: u32 = 0x6C02; const HOST_CR4: u32 = 0x6C04; const HOST_FS_BASE: u32 = 0x6C06; const HOST_GS_BASE: u32 = 0x6C08;
const HOST_TR_BASE: u32 = 0x6C0A; const HOST_GDTR_BASE: u32 = 0x6C0C; const HOST_IDTR_BASE: u32 = 0x6C0E; const HOST_SYSENTER_ESP: u32 = 0x6C10; const HOST_SYSENTER_EIP: u32 = 0x6C12;
const HOST_CS: u32 = 0xC02; const HOST_SS: u32 = 0xC04; const HOST_TR: u32 = 0xC0C;
// HOST_ES/DS/FS/GS (0xC00, 0xC06, 0xC08, 0xC0A) stay 0: their bases come from MSR fields.
const CR0_PE: u64 = 1; const CR0_PG: u64 = 1 << 31; const CR4_VMXE: u64 = 1 << 13; const CR4_PKE: u64 = 1 << 22;
const EFER_LMA: u64 = 1 << 10;

fn rdmsr(n: u32) -> u64 { unsafe { Msr::new(n).read() } }

/// The VMX capabilities, probed once: CPUID, the firmware lock, and every control the
/// mechanism needs. Logs `[drag] vmx=...` the first time.
fn caps() -> Result<Caps, Reason> {
    let mut slot = CAPS.lock();
    if let Some(c) = *slot { return c; }
    let probed = probe();
    match probed {
        Ok(c) => { serial_println!("[drag] vmx=available revision={:#x} pin={:#x} proc={:#x} proc2={:#x} exit={:#x} entry={:#x} preemption_timer={}",
            c.revision, c.pin, c.proc, c.proc2, c.exit, c.entry, c.timer_shift.is_some() as u8); }
        Err(r) => { serial_println!("[drag] vmx=unavailable reason={}", r.name()); }
    }
    *slot = Some(probed);
    probed
}

fn probe() -> Result<Caps, Reason> {
    if core::arch::x86_64::__cpuid(1).ecx & (1 << 5) == 0 { return Err(Reason::NoVmx); }
    let fc = rdmsr(0x3A);
    if fc & 1 != 0 && fc & 4 == 0 { return Err(Reason::FirmwareLocked); }
    let basic = rdmsr(0x480);
    // Write-back VMCS memory, 4 KiB or smaller regions, full physical addresses.
    if (basic >> 50) & 15 != 6 || (basic >> 32) & 0x1FFF > 4096 || basic & (1 << 48) != 0 { return Err(Reason::Basic); }
    let t = if basic & (1 << 55) != 0 { 0xC } else { 0 }; // the TRUE control MSRs
    let proc = hw::adjust(rdmsr(0x482 + t), (1 << 7) | (1 << 10) | (1 << 11) | (1 << 19) | (1 << 20) | (1 << 23) | (1 << 24) | (1 << 29) | (1u32 << 31), 0);
    // External interrupts and NMIs exit (they stay the host's); the preemption timer if there is one.
    let pin = hw::adjust(rdmsr(0x481 + t), 1 | (1 << 3), 1 << 6);
    // EPT and unrestricted guest (real mode); WBINVD exits.
    let proc2 = proc.and_then(|_| hw::adjust(rdmsr(0x48B), (1 << 1) | (1 << 6) | (1 << 7), 0));
    // 64-bit host, save and load EFER on exit; load EFER on entry.
    let exit = hw::adjust(rdmsr(0x483 + t), (1 << 9) | (1 << 20) | (1 << 21), 0);
    let entry_msr = rdmsr(0x484 + t);
    let entry = hw::adjust(entry_msr, 1 << 15, 0);
    let (Some(pin), Some(proc), Some(proc2), Some(exit), Some(entry)) = (pin, proc, proc2, exit, entry) else { return Err(Reason::Controls) };
    // EPT: 4-level walk, write-back tables, INVEPT all-context.
    let ept = rdmsr(0x48C);
    if ept & (1 << 6) == 0 || ept & (1 << 14) == 0 || ept & (1 << 20) == 0 || ept & (1 << 26) == 0 { return Err(Reason::Ept); }
    let timer_shift = (pin & (1 << 6) != 0).then(|| (rdmsr(0x485) & 31) as u8);
    Ok(Caps { revision: basic as u32 & 0x7FFF_FFFF, pin, proc, proc2, exit, entry, entry_may: (entry_msr >> 32) as u32,
        cr0: (rdmsr(0x486), rdmsr(0x487)), cr4: (rdmsr(0x488), rdmsr(0x489)), timer_shift })
}

fn held(pd: &crate::capability::ProtectionDomain) -> bool {
    matches!(pd.find_capability(abi::SLOT as u32).map(|c| c.data), Some(CapabilityData::Machine(g)) if g.holder_pd == pd.id)
}

/// Syscall 110.
pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let pd = crate::core_local::CoreLocal::get().current_pd_ref();
    let (op, a, b, c, d) = (regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8);
    let reply = if !held(pd) { [abi::DENIED, 0, 0] } else { call(pd, op, a, b, c, d) };
    regs.rsi = reply[1]; regs.rdx = reply[2];
    reply[0]
}

fn find(m: &mut Machines, pd: u32, vm: u64) -> Result<&mut Vm, u64> {
    m.vms.iter_mut().find(|v| v.id == vm && v.pd == pd).ok_or(abi::DENIED)
}
fn vcpu(m: &mut Machines, pd: u32, vm: u64, n: u64) -> Result<&mut Vcpu, u64> {
    find(m, pd, vm)?.vcpus.get_mut(n as usize).ok_or(abi::INVALID)
}

fn call(pd: &crate::capability::ProtectionDomain, op: u64, a: u64, b: u64, c: u64, d: u64) -> [u64; 3] {
    if op == abi::OP_INFO {
        return match caps() { Ok(_) => [abi::OK, 1, 0], Err(r) => [abi::OK, 0, r as u64] };
    }
    let Ok(caps) = caps() else { return [abi::UNAVAILABLE, 0, 0] };
    let result = match op {
        abi::OP_CREATE => create(pd.id),
        abi::OP_LEND_MEMORY => lend(pd, a, b, c, d),
        abi::OP_VCPU_CREATE => {
            let mut m = VMS.lock();
            find(&mut m, pd.id, a).and_then(|vm| {
                if vm.vcpus.len() >= MAX_VCPUS { return Err(abi::EXHAUSTED); }
                vm.vcpus.push(Vcpu { state: abi::State::real_mode(0, 0), event: 0, event_error: 0, running: false });
                Ok(vm.vcpus.len() as u64 - 1)
            })
        }
        abi::OP_VCPU_RUN => return run(&caps, pd.id, a, b, c, d),
        abi::OP_VCPU_GET_REGS | abi::OP_VCPU_SET_REGS => {
            let size = core::mem::size_of::<abi::State>();
            let get = op == abi::OP_VCPU_GET_REGS;
            let user_ok = if get { crate::syscalls::user_bytes_writable(c, size) } else { crate::syscalls::user_bytes_readable(c, size) };
            let mut m = VMS.lock();
            vcpu(&mut m, pd.id, a, b).and_then(|v| {
                if !user_ok || d != 0 { return Err(abi::INVALID); }
                if v.running { return Err(abi::BUSY); }
                unsafe { if get { (c as *mut abi::State).write_unaligned(v.state) } else { v.state = (c as *const abi::State).read_unaligned() } }
                Ok(0)
            })
        }
        abi::OP_INJECT => {
            let (vector, kind) = (c & 0xFF, (c >> 8) & 7);
            let valid = c >> 12 == 0 && match kind as u32 { abi::EVENT_EXTERNAL => vector >= 32, abi::EVENT_NMI => vector == 2, abi::EVENT_EXCEPTION => vector < 32, _ => false };
            let mut m = VMS.lock();
            vcpu(&mut m, pd.id, a, b).and_then(|v| {
                if !valid || d >> 32 != 0 { return Err(abi::INVALID); }
                if v.event != 0 { return Err(abi::BUSY); }
                v.event = c as u32 | (1 << 31); v.event_error = d as u32;
                Ok(0)
            })
        }
        abi::OP_DESTROY => {
            let mut m = VMS.lock();
            match m.vms.iter().position(|v| v.id == a && v.pd == pd.id) {
                None => Err(abi::DENIED),
                Some(i) if m.vms[i].vcpus.iter().any(|v| v.running) => Err(abi::BUSY),
                Some(i) => { let vm = m.vms.swap_remove(i); drop(m); free(vm); Ok(0) }
            }
        }
        _ => Err(abi::INVALID),
    };
    match result { Ok(v) => [abi::OK, v, 0], Err(s) => [s, 0, 0] }
}

fn hhdm(frame: u64) -> u64 { frame + crate::HHDM_REQUEST.response().expect("VMX_HHDM").offset }
fn entry(table: u64, i: usize) -> *mut u64 { (hhdm(table) + i as u64 * 8) as *mut u64 }

fn create(pd: u32) -> Result<u64, u64> {
    let mut m = VMS.lock();
    if m.vms.iter().filter(|v| v.pd == pd).count() >= MAX_VMS { return Err(abi::EXHAUSTED); }
    let root = crate::program_frames::take().ok_or(abi::EXHAUSTED)?;
    let id = m.next;
    m.next += 1;
    m.vms.push(Vm { id, pd, root, tables: Vec::new(), loans: Vec::new(), vcpus: Vec::new() });
    serial_println!("[drag.vm] pd={} vm={} op=create", pd, id);
    Ok(id)
}

/// Maps `pages` of the caller's own heap at `va` into the VM at `gpa`. Nothing else can
/// be lent: not kernel memory, BOOT pages, device windows or another run's frames.
fn lend(pd: &crate::capability::ProtectionDomain, vm: u64, va: u64, gpa: u64, d: u64) -> Result<u64, u64> {
    let (pages, access) = (d & 0xFFFF_FFFF, d >> 32);
    if pages == 0 || pages > 1 << 18 || access & abi::READ == 0 || access & !7 != 0 || gpa & 0xFFF != 0
        || gpa.checked_add(pages * 4096).map_or(true, |end| end > GPA_LIMIT) { return Err(abi::INVALID); }
    let mut m = VMS.lock();
    let vm = find(&mut m, pd.id, vm)?;
    let mut frames = Vec::new();
    frames.try_reserve(pages as usize).map_err(|_| abi::EXHAUSTED)?;
    crate::app_resources::heap_frames(pd, va, pages as usize, access & abi::WRITE != 0, &mut frames)?;
    for (n, &frame) in frames.iter().enumerate() {
        if let Err(e) = map(vm, gpa + n as u64 * 4096, frame, access) {
            for undo in 0..n as u64 { unmap(vm, gpa + undo * 4096); }
            return Err(e);
        }
    }
    Ok(pages)
}

fn map(vm: &mut Vm, gpa: u64, frame: u64, access: u64) -> Result<(), u64> {
    let mut table = vm.root;
    for level in (1..4).rev() {
        let e = entry(table, hw::ept_index(gpa, level));
        let v = unsafe { e.read_volatile() };
        table = if v & 7 != 0 { v & hw::ADDRESS } else {
            let t = crate::program_frames::take().ok_or(abi::EXHAUSTED)?;
            vm.tables.push(t);
            unsafe { e.write_volatile(hw::ept_table(t)) };
            t
        };
    }
    let e = entry(table, hw::ept_index(gpa, 0));
    if unsafe { e.read_volatile() } != 0 { return Err(abi::INVALID); } // already lent there
    unsafe { e.write_volatile(hw::ept_leaf(frame, access)) };
    vm.loans.push((gpa, frame));
    Ok(())
}

fn unmap(vm: &mut Vm, gpa: u64) {
    let mut table = vm.root;
    for level in (1..4).rev() {
        let v = unsafe { entry(table, hw::ept_index(gpa, level)).read_volatile() };
        if v & 7 == 0 { return; }
        table = v & hw::ADDRESS;
    }
    unsafe { entry(table, hw::ept_index(gpa, 0)).write_volatile(0) };
    vm.loans.retain(|l| l.0 != gpa);
    EPT_GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn free(vm: Vm) {
    // Every core flushes EPT translations before its next entry, before a table is reused.
    EPT_GENERATION.fetch_add(1, Ordering::AcqRel);
    for &t in vm.tables.iter().chain(core::iter::once(&vm.root)) { crate::program_frames::give(t); }
    serial_println!("[drag.vm] pd={} vm={} op=destroy pages={} tables={}", vm.pd, vm.id, vm.loans.len(), vm.tables.len() + 1);
}

/// A program run ended (`app_resources::retire`): its VMs go before its frames do.
/// No vCPU of it can be running: the run's tasks are all dead.
pub fn revoke(pd: u32) {
    let gone: Vec<Vm> = VMS.lock().vms.extract_if(.., |v| v.pd == pd).collect();
    if !gone.is_empty() { serial_println!("[drag.vm.revoked] pd={} vms={}", pd, gone.len()); }
    for vm in gone { free(vm); }
}

/// The frame is lent to one of this domain's VMs (`app_resources::code` will not seal it).
pub fn lent(pd: u32, frame: u64) -> bool {
    VMS.lock().vms.iter().any(|v| v.pd == pd && v.loans.iter().any(|l| l.1 == frame))
}

fn run(caps: &Caps, pd: u32, vm: u64, n: u64, out: u64, flags: u64) -> [u64; 3] {
    let size = core::mem::size_of::<abi::Exit>();
    if flags & !abi::RUN_LOAD_REGS != 0 || !crate::syscalls::user_bytes_writable(out, size) { return [abi::INVALID, 0, 0]; }
    let core = crate::core_local::CoreLocal::get().core_id as usize;
    if core >= CORES { return [abi::UNAVAILABLE, 0, 0]; }
    let (mut state, eptp, event, error) = {
        let mut m = VMS.lock();
        let Ok(v) = find(&mut m, pd, vm) else { return [abi::DENIED, 0, 0] };
        let eptp = hw::eptp(v.root);
        let Some(c) = v.vcpus.get_mut(n as usize) else { return [abi::INVALID, 0, 0] };
        if c.running { return [abi::BUSY, 0, 0]; }
        if flags & abi::RUN_LOAD_REGS != 0 { c.state.regs = unsafe { (out as *const abi::Exit).read_unaligned() }.regs; }
        c.running = true;
        let event = (c.event, c.event_error);
        c.event = 0;
        (c.state, eptp, event.0, event.1)
    };
    let mut exit = abi::Exit::default();
    let result = x86_64::instructions::interrupts::without_interrupts(|| unsafe { enter(caps, core, &mut state, eptp, event, error, &mut exit) });
    if let Ok(c) = vcpu(&mut VMS.lock(), pd, vm, n) {
        c.state = state;
        c.running = false;
        // An event the exit cut short is delivered again; one never entered stays queued.
        match result {
            Ok((vectoring, code)) if vectoring & (1 << 31) != 0 => { c.event = vectoring & !(1 << 12); c.event_error = code; }
            Err(_) if event != 0 => { c.event = event; c.event_error = error; }
            _ => {}
        }
    }
    match result {
        Ok(_) if crate::syscalls::user_bytes_writable(out, size) => { unsafe { (out as *mut abi::Exit).write_unaligned(exit) }; [abi::OK, 0, 0] }
        Ok(_) => [abi::INVALID, 0, 0],
        Err((status, detail)) => [status, detail, 0],
    }
}

/// One VM entry and exit on this core: VMXON, VMCS from `st`, VMLAUNCH, back, VMXOFF.
/// Restores what VMX or the guest may change and the host relies on: CR4, GDTR/IDTR
/// limits, KERNEL_GS_BASE (guest SWAPGS) and PKRU (guest WRPKRU).
unsafe fn enter(c: &Caps, core: usize, st: &mut abi::State, eptp: u64, event: u32, error: u32, exit: &mut abi::Exit) -> Result<(u32, u32), (u64, u64)> {
    use x86_64::instructions::tables::{lgdt, lidt, sgdt, sidt};
    use x86_64::registers::control::{Cr0, Cr4};
    let unavailable = (abi::UNAVAILABLE, 0);
    let (Some(vmxon), Some(vmcs)) = (region(core * 2, c.revision), region(core * 2 + 1, c.revision)) else { return Err((abi::EXHAUSTED, 0)) };
    let fc = rdmsr(0x3A);
    if fc & 1 == 0 { Msr::new(0x3A).write(fc | 1 | 4) } else if fc & 4 == 0 { return Err(unavailable); }
    let (cr0, cr4) = (Cr0::read_raw(), Cr4::read_raw());
    if !hw::fixed_ok(cr0, c.cr0.0, c.cr0.1) || !hw::fixed_ok(cr4 | CR4_VMXE, c.cr4.0, c.cr4.1) { return Err(unavailable); }
    let pkru = if cr4 & CR4_PKE != 0 { crate::pku::rdpkru() } else { 0 };
    let kernel_gs = rdmsr(0xC000_0102);
    let (gdt, idt) = (sgdt(), sidt());
    Cr4::write_raw(cr4 | CR4_VMXE);
    let mut result = Err(unavailable);
    if vmx(Op::On, vmxon) {
        result = (|| {
            if !vmx(Op::Clear, vmcs) || !vmx(Op::Load, vmcs) { return Err(unavailable); }
            let generation = EPT_GENERATION.load(Ordering::Acquire);
            if SEEN[core].swap(generation, Ordering::AcqRel) != generation { invept_all(); }
            let tss = gdt.base.as_u64() + (selector(2) & 0xFFF8) as u64;
            let tr_base = hw::descriptor_base(*(tss as *const u64), *((tss + 8) as *const u64));
            if !load(c, st, eptp, event, error, cr0, cr4 | CR4_VMXE, [gdt.base.as_u64(), idt.base.as_u64(), tr_base]) {
                return Err((abi::ENTRY_FAILED, vmread(INSTRUCTION_ERROR)));
            }
            match sexos_vmx_enter(&mut st.regs) {
                0 => Ok(save(c, st, exit)),
                1 => Err(unavailable),
                _ => Err((abi::ENTRY_FAILED, vmread(INSTRUCTION_ERROR))),
            }
        })();
        vmx(Op::Clear, vmcs);
        core::arch::asm!("vmxoff", options(nostack));
    }
    Cr4::write_raw(cr4);
    lgdt(&gdt);
    lidt(&idt);
    Msr::new(0xC000_0102).write(kernel_gs);
    if cr4 & CR4_PKE != 0 { crate::pku::wrpkru(pkru); }
    // An NMI that exited belongs to the host (the stop screen uses them).
    if result.is_ok() && exit.reason == abi::exit::EXCEPTION_NMI && exit.interruption_info & 0x8000_0700 == 0x8000_0200 {
        core::arch::asm!("int 2");
    }
    result
}

/// Writes the whole VMCS: host state of this core, controls, and the guest from `st`.
unsafe fn load(c: &Caps, st: &abi::State, eptp: u64, event: u32, error: u32, cr0: u64, cr4: u64, bases: [u64; 3]) -> bool {
    let mut ok = true;
    let mut w = |f: u32, v: u64| ok &= vmwrite(f, v);
    let global_root: u64;
    core::arch::asm!("mov {}, cr3", out(reg) global_root, options(nomem, nostack));
    w(HOST_CR0, cr0); w(HOST_CR3, global_root); w(HOST_CR4, cr4);
    w(HOST_CS, selector(0) as u64 & 0xFFF8); w(HOST_SS, selector(1) as u64 & 0xFFF8); w(HOST_TR, selector(2) as u64 & 0xFFF8);
    w(HOST_FS_BASE, rdmsr(0xC000_0100)); w(HOST_GS_BASE, rdmsr(0xC000_0101));
    w(HOST_GDTR_BASE, bases[0]); w(HOST_IDTR_BASE, bases[1]); w(HOST_TR_BASE, bases[2]);
    w(HOST_SYSENTER_CS, rdmsr(0x174)); w(HOST_SYSENTER_ESP, rdmsr(0x175)); w(HOST_SYSENTER_EIP, rdmsr(0x176));
    w(HOST_EFER, rdmsr(0xC000_0080));
    let long = if st.efer & EFER_LMA != 0 && c.entry_may & (1 << 9) != 0 { 1 << 9 } else { 0 };
    w(PIN, c.pin as u64); w(PROC, c.proc as u64); w(PROC2, c.proc2 as u64); w(EXIT_CTL, c.exit as u64); w(ENTRY_CTL, (c.entry | long) as u64);
    // Exception bitmap, #PF masks, CR3 targets, MSR load/store lists, entry length: none.
    for f in [0x4004, 0x4006, 0x4008, 0x400A, 0x400E, 0x4010, 0x4014, 0x401A] { w(f, 0); }
    w(EPT_POINTER, eptp); w(LINK, !0);
    w(ENTRY_EVENT, event as u64); w(ENTRY_ERROR, error as u64);
    if let Some(shift) = c.timer_shift { w(TIMER, SLICE_TSC >> shift); }
    // CR0/CR4 bits VMX forces stay hidden behind the read shadows.
    let forced = c.cr0.0 & !(CR0_PE | CR0_PG);
    w(CR0_MASK, forced); w(CR0_SHADOW, st.cr0); w(GUEST_CR0, (st.cr0 | forced) & c.cr0.1);
    w(CR4_MASK, c.cr4.0); w(CR4_SHADOW, st.cr4); w(GUEST_CR4, (st.cr4 | c.cr4.0) & c.cr4.1);
    w(GUEST_CR3, st.guest_cr3); w(GUEST_EFER, st.efer); w(GUEST_DR7, 0x400); w(GUEST_DEBUGCTL, 0);
    w(GUEST_RSP, st.regs.rsp); w(GUEST_RIP, st.regs.rip); w(GUEST_RFLAGS, (st.regs.rflags & 0x003F_7FD5) | 2);
    for (i, s) in st.segments.iter().enumerate() {
        let f = hw::segment_fields(i as u32);
        w(f[0], s.selector as u64); w(f[1], s.limit as u64); w(f[2], s.access as u64); w(f[3], s.base);
    }
    w(GDTR_BASE, st.gdtr_base); w(IDTR_BASE, st.idtr_base); w(GDTR_LIMIT, st.gdtr_limit as u64); w(IDTR_LIMIT, st.idtr_limit as u64);
    w(INTERRUPTIBILITY, st.interruptibility as u64); w(ACTIVITY, st.activity.min(1) as u64);
    for f in [0x6822, 0x482A, 0x6824, 0x6826] { w(f, 0); } // pending debug, guest SYSENTER
    ok
}

/// Reads the guest back into `st` and the exit into `exit`. Returns the IDT-vectoring
/// event the exit interrupted, to deliver again.
unsafe fn save(c: &Caps, st: &mut abi::State, exit: &mut abi::Exit) -> (u32, u32) {
    st.regs.rsp = vmread(GUEST_RSP); st.regs.rip = vmread(GUEST_RIP); st.regs.rflags = vmread(GUEST_RFLAGS);
    let forced = c.cr0.0 & !(CR0_PE | CR0_PG);
    st.cr0 = (vmread(GUEST_CR0) & !forced) | (st.cr0 & forced);
    st.cr4 = (vmread(GUEST_CR4) & !c.cr4.0) | (st.cr4 & c.cr4.0);
    st.guest_cr3 = vmread(GUEST_CR3); st.efer = vmread(GUEST_EFER);
    for (i, s) in st.segments.iter_mut().enumerate() {
        let f = hw::segment_fields(i as u32);
        s.selector = vmread(f[0]) as u16; s.limit = vmread(f[1]) as u32; s.access = vmread(f[2]) as u32; s.base = vmread(f[3]);
    }
    st.gdtr_base = vmread(GDTR_BASE); st.idtr_base = vmread(IDTR_BASE);
    st.gdtr_limit = vmread(GDTR_LIMIT) as u32; st.idtr_limit = vmread(IDTR_LIMIT) as u32;
    st.interruptibility = vmread(INTERRUPTIBILITY) as u32; st.activity = vmread(ACTIVITY) as u32;
    let reason = vmread(REASON) as u32;
    *exit = abi::Exit { reason: reason & 0x8000_FFFF, instruction_len: vmread(INSTRUCTION_LEN) as u32,
        qualification: vmread(QUALIFICATION), guest_physical: vmread(GUEST_PHYSICAL), guest_linear: vmread(GUEST_LINEAR),
        instruction_info: vmread(INSTRUCTION_INFO) as u32, interruption_info: vmread(EXIT_EVENT) as u32,
        interruption_error: vmread(EXIT_EVENT_ERROR) as u32, regs: st.regs, ..abi::Exit::default() };
    if reason & 0xFFFF == abi::exit::IO {
        (exit.port, exit.io_size, exit.io_flags) = hw::io(exit.qualification);
    }
    if reason & (1 << 31) != 0 { return (0, 0); }
    (vmread(IDT_VECTORING) as u32, vmread(IDT_VECTORING_ERROR) as u32)
}

/// This core's VMXON region (even) or VMCS (odd), taken on first use and kept.
fn region(i: usize, revision: u32) -> Option<u64> {
    let held = REGIONS[i].load(Ordering::Acquire);
    if held != 0 { return Some(held); }
    let frame = crate::program_frames::take()?;
    unsafe { (hhdm(frame) as *mut u32).write_volatile(revision) };
    REGIONS[i].store(frame, Ordering::Release);
    Some(frame)
}

enum Op { On, Clear, Load }
unsafe fn vmx(op: Op, frame: u64) -> bool {
    let p: *const u64 = &frame;
    let f: u8;
    match op {
        Op::On => core::arch::asm!("vmxon qword ptr [{p}]", "setna {f}", p = in(reg) p, f = out(reg_byte) f, options(nostack)),
        Op::Clear => core::arch::asm!("vmclear qword ptr [{p}]", "setna {f}", p = in(reg) p, f = out(reg_byte) f, options(nostack)),
        Op::Load => core::arch::asm!("vmptrld qword ptr [{p}]", "setna {f}", p = in(reg) p, f = out(reg_byte) f, options(nostack)),
    }
    f == 0
}
unsafe fn vmwrite(field: u32, value: u64) -> bool {
    let f: u8;
    core::arch::asm!("vmwrite {0}, {1}", "setna {2}", in(reg) field as u64, in(reg) value, out(reg_byte) f, options(nostack));
    f == 0
}
unsafe fn vmread(field: u32) -> u64 {
    let v: u64;
    core::arch::asm!("vmread {0}, {1}", out(reg) v, in(reg) field as u64, options(nostack));
    v
}
unsafe fn invept_all() {
    let descriptor = [0u64; 2];
    core::arch::asm!("invept {0}, xmmword ptr [{1}]", in(reg) 2u64, in(reg) &descriptor, options(nostack));
}
/// CS (0), SS (1) or TR (2) of this core.
fn selector(which: u8) -> u16 {
    let v: u16;
    unsafe { match which {
        0 => core::arch::asm!("mov {0:x}, cs", out(reg) v, options(nomem, nostack)),
        1 => core::arch::asm!("mov {0:x}, ss", out(reg) v, options(nomem, nostack)),
        _ => core::arch::asm!("str {0:x}", out(reg) v, options(nomem, nostack)),
    } }
    v
}

extern "C" { fn sexos_vmx_enter(regs: *mut abi::Regs) -> u64; }
// Loads the guest's general registers from `regs` (rdi) and launches. The VM exit lands at
// sexos_vmx_exit with the stack as it was at VMLAUNCH (HOST_RSP), saves them back and
// returns 0. A launch that fails returns 1 (VMfailInvalid) or 2 (VMfailValid).
core::arch::global_asm!(r#"
.global sexos_vmx_enter
sexos_vmx_enter:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    push rdi
    mov rax, 0x6C14
    vmwrite rax, rsp
    lea rcx, [rip + sexos_vmx_exit]
    mov rax, 0x6C16
    vmwrite rax, rcx
    mov rax, [rdi]
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
    vmlaunch
    pop rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    mov eax, 1
    jc sexos_vmx_failed
    mov eax, 2
sexos_vmx_failed:
    ret
sexos_vmx_exit:
    push rdi
    mov rdi, [rsp + 8]
    mov [rdi], rax
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
    pop rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    xor eax, eax
    ret
"#);
