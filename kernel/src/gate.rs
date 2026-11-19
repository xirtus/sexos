// SPDX-License-Identifier: GPL-3.0-only
//! PKU gates, kernel side (docs/handoff/PKU_GATES_STUDY_2026_11_18.md, P6.2; ABI in
//! `sex_pdx::gate`, syscall 111).
//!
//! A gate stub finds the running task's gate area with RDPID, which reads IA32_TSC_AUX.
//! `Scheduler::switch_to`, the one place a core starts running a task, loads TSC_AUX
//! from `TaskContext.gate_aux` once [`ON`] is set. Ring 3 cannot write TSC_AUX, and
//! Drag guests get neither RDTSCP nor the MSR, so a task's RDPID names its own area.
//!
//! The kernel owns everything a stub trusts: the stub pages (built by `sex_gate`, user
//! read+execute, key 0), the matrix page (one byte per stub, user read-only) and each
//! task's area: a frame page under [`FRAME_KEY`] (readable by every domain, writable
//! only under a stub's G value) and one slot page per domain the task enters, under
//! that domain's key, with the callee's stack top in it. The kernel writes all of them
//! through the HHDM alias, which PKU does not cover. Stubs live below the service code
//! (`loader::elf::pd_code_base`), so every entry is in rel32 reach.
//!
//! A Gate may also be exported under a number; a service holding a route (Domain
//! capability) to its owner binds it with OP_BIND_ROUTE.
//!
//! Not yet: nested gate calls (a callee calling a third service on the caller's task),
//! freeing areas when a task ends, and unwinding a caller whose callee dies (P6.2d).
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;
use x86_64::structures::paging::PageTableFlags as F;
use x86_64::VirtAddr;
use crate::capability::{CapabilityData, ProtectionDomain};
use crate::capability_table::{rights, ObjectCap, OBJECTS};
use sex_pdx::endpoint::{E_CLOSED, E_FULL, E_RIGHTS};
use sex_pdx::gate::*;

pub const TSC_AUX: u32 = 0xC000_0103;
/// The gate frame key. Program rows gave it up for protection views; until they all
/// have, [`KEY_FREE`] keeps it closed to every domain and gates unavailable.
pub const FRAME_KEY: u8 = 5;
pub const KEY_FREE: bool = {
    use sex_pdx::organs::program_key as k;
    k(0) != FRAME_KEY && k(1) != FRAME_KEY && k(2) != FRAME_KEY
};

const PAGE: u64 = 4096;
const MATRIX: u64 = 0x3EFF_F000;
const STUBS: u64 = 0x3F00_0000;
const STUB_BYTES: u64 = 1024;
const MAX_STUBS: usize = 1024;
const MAX_AREAS: u64 = 1024;
const CALLEE_STACK_PAGES: u64 = 4;
const _: () = assert!(sex_gate::template::LEN as u64 <= STUB_BYTES && MAX_STUBS < PAGE as usize);
const _: () = assert!(sex_gate::AREA_SHIFT == 18 && STUBS + MAX_STUBS as u64 * STUB_BYTES <= 0x4000_0000);

/// Set by the first bind; from then on every switch loads TSC_AUX.
pub static ON: AtomicBool = AtomicBool::new(false);
static USABLE: AtomicBool = AtomicBool::new(false);
static AREAS: AtomicU64 = AtomicU64::new(0);

struct Gate { object: u32, generation: u32, callee: u32, entry: u64, export: u64 }
struct Bound { object: u32, generation: u32, caller: u32 }
struct State {
    gates: Vec<Gate>,
    /// By stub id - 1.
    stubs: Vec<Bound>,
    /// Frames of the stub pages, in VA order from `STUBS`.
    pages: Vec<u64>,
    matrix: u64,
    /// By area index: the slot pages mapped in it, one bit per domain (index 0 unused).
    areas: Vec<u64>,
}
static STATE: Mutex<State> = Mutex::new(State { gates: Vec::new(), stubs: Vec::new(), pages: Vec::new(), matrix: 0, areas: Vec::new() });

fn rdpid() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("rdpid {}", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

/// Boot check on the first core: RDPID exists and returns what was written to TSC_AUX.
pub fn init() -> bool {
    let has = core::arch::x86_64::__cpuid_count(7, 0).ecx & (1 << 22) != 0;
    let ok = has && crate::pku::PKU_ENABLED.load(Ordering::Acquire) != 0 && unsafe {
        let mut m = x86_64::registers::model_specific::Msr::new(TSC_AUX);
        m.write(0x5A5);
        let seen = rdpid();
        m.write(0);
        seen == 0x5A5
    };
    USABLE.store(ok, Ordering::Release);
    crate::serial_println!("[gate.rdpid] cpu={} usable={} frame_key_free={} gates=off", has as u8, ok as u8, KEY_FREE as u8);
    ok
}

fn hhdm() -> u64 { crate::HHDM_REQUEST.response().expect("GATE_NO_HHDM").offset }

/// A fresh frame filled with `fill`, mapped user-mode at `va` under `key`.
fn page(va: u64, key: u8, flags: F, fill: u8) -> Result<u64, u64> {
    let frame = crate::memory::allocator::alloc_frame().ok_or(E_FULL)?;
    unsafe { core::ptr::write_bytes((frame + hhdm()) as *mut u8, fill, PAGE as usize) };
    let mut vas = crate::memory::manager::GLOBAL_VAS.lock();
    vas.as_mut().ok_or(E_FULL)?
        .map_physical_range(VirtAddr::new(va), frame, PAGE, F::PRESENT | F::USER_ACCESSIBLE | flags, key)
        .map_err(|_| E_FULL)?;
    Ok(frame)
}

fn put(frame: u64, offset: u64, value: u64) {
    unsafe { ((frame + hhdm() + offset) as *mut u64).write_volatile(value) }
}

fn me() -> &'static ProtectionDomain { crate::core_local::CoreLocal::get().current_pd_ref() }
fn table(pd: &ProtectionDomain) -> &'static crate::capability_table::CapabilityTable { unsafe { &*pd.cap_table } }
fn live(object: u32, generation: u32) -> bool { OBJECTS.live(&ObjectCap { object, generation, rights: 0, badge: 0 }) }

/// Gates join trusted services only: never a program row, a record or the browser.
fn trusted(pd: &ProtectionDomain) -> bool {
    use sex_pdx::organs::{program_key, RECORD_KEY, SHARED_PROGRAM_KEY};
    let k = pd.pku_key;
    pd.id >= 1 && pd.id <= sex_gate::MAX_DOMAIN && k != 0 && k != FRAME_KEY && k != RECORD_KEY
        && k != SHARED_PROGRAM_KEY && k != program_key(0) && k != program_key(2) && k != crate::pku::UNTRUSTED
}

/// Stubs whose Gate died (destroyed, or its owner's incarnation ended) deny.
fn sweep(st: &mut State) {
    st.gates.retain(|g| live(g.object, g.generation));
    if st.matrix == 0 { return; }
    for (i, b) in st.stubs.iter().enumerate() {
        if !live(b.object, b.generation) { unsafe { ((st.matrix + hhdm() + i as u64 + 1) as *mut u8).write_volatile(0) } }
    }
}

/// OP_CREATE: a Gate into the caller's own code at `entry`, exported as `export`.
fn create(entry: u64, export: u64) -> Result<u64, u64> {
    let pd = me();
    if !trusted(pd) || export > 255 { return Err(E_RIGHTS); }
    let pte = crate::memory::manager::read_pte_flags(VirtAddr::new(entry)).map_err(|_| E_RIGHTS)?;
    if pte & F::USER_ACCESSIBLE.bits() == 0 || pte & F::NO_EXECUTE.bits() != 0 || ((pte >> 59) & 0xF) as u8 != pd.pku_key {
        return Err(E_RIGHTS);
    }
    let cap = OBJECTS.create(pd.id).ok_or(E_FULL)?;
    let mut st = STATE.lock();
    sweep(&mut st);
    if export != 0 && st.gates.iter().any(|g| g.callee == pd.id && g.export == export) {
        OBJECTS.destroy(cap.object);
        return Err(sex_pdx::endpoint::E_BUSY);
    }
    st.gates.push(Gate { object: cap.object, generation: cap.generation, callee: pd.id, entry, export });
    match table(pd).install(CapabilityData::Gate(cap)) {
        Ok(slot) => Ok(slot as u64),
        Err(_) => { OBJECTS.destroy(cap.object); st.gates.pop(); Err(E_FULL) }
    }
}

/// OP_DESTROY: the owner ends a Gate; its stubs deny from now on.
fn destroy(slot: u32) -> Result<u64, u64> {
    let pd = me();
    let Some(CapabilityData::Gate(cap)) = table(pd).find(slot).map(|c| c.data) else { return Err(sex_pdx::ERR_CAP_INVALID) };
    if !OBJECTS.live(&cap) { return Err(E_CLOSED); }
    if OBJECTS.owner(cap.object) != Some(pd.id) { return Err(E_RIGHTS); }
    OBJECTS.destroy(cap.object);
    table(pd).revoke(slot);
    sweep(&mut STATE.lock());
    Ok(0)
}

/// OP_BIND: the Gate capability in `slot`. OP_BIND_ROUTE: the Gate the service behind
/// the route in `slot` exported as `export`; holding the route is the authority.
fn bind(slot: u32, route: Option<u64>) -> Result<u64, u64> {
    if !KEY_FREE || !USABLE.load(Ordering::Acquire) { return Err(UNAVAILABLE); }
    let caller = me();
    if !trusted(caller) { return Err(E_RIGHTS); }
    let mut st = STATE.lock();
    sweep(&mut st);
    let found = match route {
        None => {
            let Some(CapabilityData::Gate(cap)) = table(caller).find(slot).map(|c| c.data) else { return Err(sex_pdx::ERR_CAP_INVALID) };
            if !OBJECTS.live(&cap) { return Err(E_CLOSED); }
            if !cap.allows(rights::CALL) { return Err(E_RIGHTS); }
            st.gates.iter().find(|g| g.object == cap.object && g.generation == cap.generation)
        }
        Some(export) => {
            let held = crate::capability_slot::decode_capability_slot(slot as u64).and_then(|s| table(caller).find(s));
            let Some(CapabilityData::Domain(target)) = held.map(|c| c.data) else { return Err(sex_pdx::ERR_CAP_INVALID) };
            st.gates.iter().find(|g| g.callee == target && export != 0 && g.export == export)
        }
    };
    let (object, generation, callee, entry) = found.map(|g| (g.object, g.generation, g.callee, g.entry)).ok_or(E_CLOSED)?;
    let cap = ObjectCap { object, generation, rights: 0, badge: 0 };
    let callee = crate::ipc::DOMAIN_REGISTRY.get(callee).ok_or(E_CLOSED)?;
    if !trusted(callee) || callee.pku_key == caller.pku_key { return Err(E_RIGHTS); }
    let areas = match AREAS.load(Ordering::Acquire) {
        0 => {
            let base = crate::memory::va_allocator::allocate_va(((MAX_AREAS + 1) << sex_gate::AREA_SHIFT) as usize).ok_or(E_FULL)?;
            AREAS.store(base, Ordering::Release);
            base
        }
        base => base,
    };
    if st.matrix == 0 { st.matrix = page(MATRIX, 0, F::NO_EXECUTE, 0)?; }
    let found = st.stubs.iter().position(|b| b.object == cap.object && b.generation == cap.generation && b.caller == caller.id);
    let id = match found {
        Some(i) => i as u32 + 1,
        None => {
            if st.stubs.len() >= MAX_STUBS { return Err(E_FULL); }
            let id = st.stubs.len() as u32 + 1;
            let at = STUBS + (id as u64 - 1) * STUB_BYTES;
            let code = sex_gate::build(&sex_gate::Stub {
                at, matrix: MATRIX + id as u64, id, frame_key: FRAME_KEY,
                caller_pkru: caller.base_pkru_mask, callee_pkru: callee.base_pkru_mask,
                caller: caller.id, callee: callee.id, entry, areas,
            }).map_err(|_| E_RIGHTS)?;
            let index = ((at - STUBS) / PAGE) as usize;
            while st.pages.len() <= index {
                let va = STUBS + st.pages.len() as u64 * PAGE;
                let frame = page(va, 0, F::empty(), 0xCC)?; // int3 between stubs
                st.pages.push(frame);
            }
            unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), (st.pages[index] + hhdm() + at % PAGE) as *mut u8, code.len()) };
            st.stubs.push(Bound { object: cap.object, generation: cap.generation, caller: caller.id });
            id
        }
    };
    area(&mut st, areas, caller, callee)?;
    unsafe { ((st.matrix + hhdm() + id as u64) as *mut u8).write_volatile(1) };
    ON.store(true, Ordering::Release);
    Ok(STUBS + (id as u64 - 1) * STUB_BYTES)
}

/// This task's gate area, with slot pages for `caller` and `callee` (and the callee's
/// stack for this task). The first bind also points this core's TSC_AUX at it.
fn area(st: &mut State, areas: u64, caller: &ProtectionDomain, callee: &ProtectionDomain) -> Result<(), u64> {
    let core = crate::core_local::CoreLocal::get().core_id as usize;
    let task = crate::scheduler::SCHEDULERS[core].current_task.load(Ordering::Acquire);
    if task.is_null() { return Err(E_RIGHTS); }
    let ctx = unsafe { &mut (*task).context };
    if ctx.pd_id as u32 != caller.id { return Err(E_RIGHTS); }
    if st.areas.is_empty() { st.areas.push(!0); }
    if ctx.gate_aux == 0 {
        let index = st.areas.len() as u64;
        if index > MAX_AREAS { return Err(E_FULL); }
        let frame = page(areas + (index << sex_gate::AREA_SHIFT), FRAME_KEY, F::WRITABLE | F::NO_EXECUTE, 0)?;
        put(frame, sex_gate::frame::CUR as u64, caller.base_pkru_mask as u64);
        put(frame, sex_gate::frame::ACTIVE as u64, 1 << caller.id);
        st.areas.push(0);
        ctx.gate_aux = index;
        unsafe { x86_64::registers::model_specific::Msr::new(TSC_AUX).write(index) };
    }
    let index = ctx.gate_aux as usize;
    let base = areas + ((index as u64) << sex_gate::AREA_SHIFT);
    for (pd, stack) in [(caller, false), (callee, true)] {
        if st.areas[index] & (1 << pd.id) != 0 { continue; }
        let slot = page(base + sex_gate::slot_offset(pd.id) as u64, pd.pku_key, F::WRITABLE | F::NO_EXECUTE, 0)?;
        if stack {
            // One unmapped guard page below the callee's stack.
            let va = crate::memory::va_allocator::allocate_va(((CALLEE_STACK_PAGES + 1) * PAGE) as usize).ok_or(E_FULL)?;
            for p in 1..=CALLEE_STACK_PAGES { page(va + p * PAGE, pd.pku_key, F::WRITABLE | F::NO_EXECUTE, 0)?; }
            put(slot, sex_gate::slot::STACK as u64, va + (CALLEE_STACK_PAGES + 1) * PAGE);
        }
        st.areas[index] |= 1 << pd.id;
    }
    Ok(())
}

/// SYS_GATE (111): rdi = op | slot << 8, rsi = argument; rsi out = value.
pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let slot = (regs.rdi >> 8) as u32;
    let result = match regs.rdi & 0xFF {
        OP_CREATE => create(regs.rsi, regs.rdx),
        OP_BIND => bind(slot, None),
        OP_BIND_ROUTE => bind(slot, Some(regs.rsi)),
        OP_DESTROY => destroy(slot),
        _ => Err(sex_pdx::ERR_CAP_INVALID),
    };
    match result { Ok(v) => { regs.rsi = v; 0 } Err(e) => { regs.rsi = 0; e } }
}
