// SPDX-License-Identifier: GPL-3.0-only
//! A service that ends starts again as it first was. BOOT keeps a copy of every page
//! the service owns before it first runs; a restart puts those pages back, forgets the
//! old run's messages and replies, and points the same task at the same entry. The
//! domain, its key and every capability it holds stay as BOOT made them: nothing is
//! minted, so a restart can never grant more than the first run had.
use core::sync::atomic::{AtomicU32, Ordering};
use spin::Mutex;

/// Code and data of one service fit its 16 MiB slot; the stack is 64 KiB under its top.
const SLOT_BYTES: u64 = 0x0100_0000;
const STACK_BYTES: u64 = 64 * 1024;
const MAX_PAGES: usize = 2048;
/// A service that keeps dying is left down rather than restarted forever.
const MAX_RESTARTS: u32 = 8;
const PTE_FRAME: u64 = 0x000f_ffff_ffff_f000;

#[derive(Clone, Copy)]
struct Page { frame: u64, pristine: u64 }
struct Service { pd: u32, entry: u64, stack_top: u64, pages: [Page; MAX_PAGES], count: usize }
const SERVICES: usize = 6;
static SERVICE: [Mutex<Service>; SERVICES] = [const { Mutex::new(Service { pd: 0, entry: 0, stack_top: 0, pages: [Page { frame: 0, pristine: 0 }; MAX_PAGES], count: 0 }) }; SERVICES];
// BOOT publishes immutable identity separately; self count cannot contend with its own rearm.
static SERVICE_IDS:[AtomicU32;SERVICES]=[const{AtomicU32::new(0)};SERVICES];
static RESTARTS: [AtomicU32; SERVICES] = [const { AtomicU32::new(0) }; SERVICES];
static GAVE_UP: [core::sync::atomic::AtomicBool; SERVICES] = [const { core::sync::atomic::AtomicBool::new(false) }; SERVICES];

fn code_base(domain_id: u8) -> u64 { 0x4000_0000 + (domain_id as u64).saturating_sub(1) * SLOT_BYTES }
fn stack_top(domain_id: u8) -> u64 { 0x7000_0000_0000 + (domain_id as u64).saturating_sub(1) * SLOT_BYTES + 0x0010_0000 }

/// At BOOT, before the service first runs: remember every page it owns as it is now.
pub fn capture(pd_id: u32, domain_id: u8) {
    if pd_id == 0 { return; }
    let pd = crate::ipc::DOMAIN_REGISTRY.get(pd_id).expect("SERVICE_RESTART_DOMAIN");
    let main = pd.main_task.load(Ordering::Acquire);
    assert!(!main.is_null(), "SERVICE_RESTART_TASK");
    let (index,mut s)=SERVICE.iter().enumerate().map(|(i,m)|(i,m.lock())).find(|(_,s)|s.pd==0).expect("SERVICE_RESTART_SLOTS");
    let (base, top) = (code_base(domain_id), stack_top(domain_id));
    let ranges = [(base, base + SLOT_BYTES), (top - STACK_BYTES, top)];
    for (start, end) in ranges {
        for va in (start..end).step_by(4096) {
            let Some(leaf) = crate::boot_memory::existing_leaf_address(va) else { continue };
            let value = unsafe { core::ptr::read_volatile(leaf as *const u64) };
            // Present, user, on this service's key: its own page.
            if value & 5 != 5 || (value >> 59) & 15 != pd.pku_key as u64 { continue; }
            assert!(s.count < MAX_PAGES, "SERVICE_RESTART_PAGES");
            let n = s.count;
            s.pages[n] = Page { frame: value & PTE_FRAME, pristine: 0 };
            s.count += 1;
        }
    }
    // One reservation holds every copy.
    let count = s.count;
    let copies = crate::boot_memory::backing(count as u64 * 4096);
    for (i, page) in s.pages[..count].iter_mut().enumerate() {
        page.pristine = copies + i as u64 * 4096;
        crate::boot_memory::copy_frame(page.frame, page.pristine);
    }
    s.pd = pd_id;
    s.entry = unsafe { (*main).context.rip };
    s.stack_top = unsafe { (*main).context.rsp };
    SERVICE_IDS[index].store(pd_id,Ordering::Release);
    crate::serial_println!("[boot.service.restartable] pd={} task={} pages={} entry={:#x}", pd_id, unsafe{(*main).id}, s.count, s.entry);
}

/// A service keeps `pages` of its own pages from `va` across its restarts: a restart
/// leaves them as the last run wrote them, so the next run can pick its session up.
/// Only pages BOOT captured for this very service, at most 4.
pub fn keep(pd: u32, va: u64, pages: u64) -> bool {
    if pd == 0 || va & 0xfff != 0 || !(1..=4).contains(&pages) { return false; }
    let Some(mut s) = SERVICE.iter().map(|m| m.lock()).find(|s| s.pd == pd) else { return false };
    let mut found = [usize::MAX; 4];
    for (n, slot) in found.iter_mut().enumerate().take(pages as usize) {
        let Some(leaf) = crate::boot_memory::existing_leaf_address(va + n as u64 * 4096) else { return false };
        let frame = unsafe { core::ptr::read_volatile(leaf as *const u64) } & PTE_FRAME;
        let count = s.count;
        let Some(i) = s.pages[..count].iter().position(|p| p.frame == frame) else { return false };
        *slot = i;
    }
    for i in found.into_iter().filter(|&i| i != usize::MAX) { s.pages[i].pristine = 0; }
    true
}

/// How many times the service in domain `pd` has been started again.
pub fn restarts_of(pd: u32) -> u32 {
    SERVICE_IDS.iter().position(|id|pd!=0&&id.load(Ordering::Acquire)==pd)
        .map_or(0,|i|RESTARTS[i].load(Ordering::Acquire))
}

/// Snapshot only a genuinely held route, under the same lock as peer rearm.
pub fn peer_state(holder:&crate::capability::ProtectionDomain,slot:u64)->Result<sex_pdx::crash_log::PeerState,u64>{
    use sex_pdx::crash_log::{self as w,PeerLife as L,PeerState};
    use crate::scheduler::task_ownership::Phase;
    let slot=crate::capability_slot::decode_capability_slot(slot).ok_or(w::INVALID)?;
    let target=match holder.find_capability(slot).map(|c|c.data){Some(crate::capability::CapabilityData::Domain(pd))=>pd,_=>return Err(w::DENIED)};
    let mut contended=false;
    for index in 0..SERVICES{
        let Some(service)=SERVICE[index].try_lock()else{contended=true;continue};
        if service.pd==0 || service.pd!=target{continue}
        let domain=crate::ipc::DOMAIN_REGISTRY.get(target).ok_or(w::NOT_FOUND)?;
        let main=domain.main_task.load(Ordering::Acquire);
        let main=unsafe{main.as_ref()}.ok_or(w::INVALID)?;
        if main.id==0 || main.context.pd_id!=target as u64 || main.context.pd_ptr!=domain as *const _{return Err(w::INVALID)}
        let state=main.state.snapshot();
        let ending=target<64 && END_REQUESTS.load(Ordering::Acquire)&(1u64<<target)!=0;
        let life=if GAVE_UP[index].load(Ordering::Acquire){L::GaveUp}
            else if state.exited{if state.phase==Phase::Dead&&state.owner_core.is_none(){L::Dead}else{L::Exited}}
            else if state.phase==Phase::Dead{return Err(w::INVALID)}
            else if ending{L::Ending}else if state.fault_blocked{L::FaultBlocked}
            else if state.phase==Phase::Constructed{L::Starting}else{L::Live};
        let epoch=(RESTARTS[index].load(Ordering::Acquire) as u64).checked_add(1).ok_or(w::INVALID)?;
        return Ok(PeerState{epoch,life})
    }
    Err(if contended{w::RETRY}else{w::NOT_FOUND})
}

/// Domains asked to end at their next syscall, as a fault would end them (one bit each).
static END_REQUESTS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Ask domain `pd` to end at its next syscall, and wake it so it makes one.
pub fn request_end(pd: u32) {
    if pd == 0 || pd >= 64 { return; }
    END_REQUESTS.fetch_or(1 << pd, Ordering::AcqRel);
    if let Some(domain) = crate::ipc::DOMAIN_REGISTRY.get(pd) {
        crate::scheduler::unpark_thread(domain.main_task.load(Ordering::Acquire));
    }
}
/// From syscall entry: whether the caller was asked to end (and forget the request).
pub fn end_requested(pd: u32) -> bool {
    if pd == 0 || pd >= 64 || END_REQUESTS.load(Ordering::Relaxed) & (1 << pd) == 0 { return false; }
    END_REQUESTS.fetch_and(!(1 << pd), Ordering::AcqRel) & (1 << pd) != 0
}

/// Display and Shell hold the whole desktop. Both start again in place; one that
/// keeps dying is given up on, and then the machine stops (screen, record,
/// restart) instead of leaving a frozen desktop.
const DESKTOP: [u32; 2] = [1, 3];
const SHELL_PD: u32 = 3;
const DISPLAY_PD: u32 = 1;
static DESKTOP_STOPPED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// From the timer, IF masked: a service whose task has ended starts again.
pub fn poll() {
    for index in 0..SERVICES { poll_one(index); }
    // New crash records (a fault, this restart) go to Files for the disk ring.
    crate::crash_log::poll();
    for pd_id in DESKTOP {
        let mut held = false;
        let slot = (0..SERVICES).find(|&i| match SERVICE[i].try_lock() { Some(s) => s.pd == pd_id, None => { held = true; false } });
        if held || slot.is_some_and(|i| !GAVE_UP[i].load(Ordering::Relaxed)) { continue; }
        let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(pd_id) else { continue };
        let main = pd.main_task.load(Ordering::Acquire);
        if main.is_null() { continue; }
        let state = unsafe { (*main).state.snapshot() };
        if state.exited && state.phase == crate::scheduler::task_ownership::Phase::Dead
            && !DESKTOP_STOPPED.swap(true, Ordering::AcqRel) {
            crate::console::stop(format_args!("{} ended and keeps ending. The desktop cannot go on without it, so the machine restarts.",
                sex_pdx::crash_log::domain_name(pd_id)));
        }
    }
}
fn poll_one(index: usize) {
    let Some(s) = SERVICE[index].try_lock() else { return };
    if s.pd == 0 || GAVE_UP[index].load(Ordering::Relaxed) { return; }
    let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(s.pd) else { return };
    let main = pd.main_task.load(Ordering::Acquire);
    if main.is_null() { return; }
    let state = unsafe { (*main).state.snapshot() };
    if !state.exited || state.phase != crate::scheduler::task_ownership::Phase::Dead || state.owner_core.is_some() { return; }
    if RESTARTS[index].load(Ordering::Relaxed) >= MAX_RESTARTS {
        GAVE_UP[index].store(true, Ordering::Relaxed);
        crate::crash_log::service(s.pd, MAX_RESTARTS, true);
        return;
    }
    crate::pci_command::quiesce(pd);
    crate::iommu::revoke(s.pd);
    for page in s.pages[..s.count].iter().filter(|p| p.pristine != 0) { crate::boot_memory::copy_frame(page.pristine, page.frame); }
    // Requests to the dead run, and answers it was owed, belong to that run.
    let mut dropped = 0;
    while unsafe { (*pd.message_ring).dequeue() }.is_some() { dropped += 1; if dropped > 256 { break; } }
    while unsafe { (*pd.signal_ring).dequeue() }.is_some() {}
    crate::ipc::messages::IpcReply::erase_queue(&mut pd.incoming_replies.lock());
    pd.reply_credits.lock().clear();
    let id = unsafe { (*main).id };
    unsafe { (*main).rearm(id, s.entry, s.stack_top, pd); }
    let count = RESTARTS[index].fetch_add(1, Ordering::Relaxed) + 1;
    let admitted = crate::scheduler::SCHEDULERS[0].activate_task(main);
    crate::serial_println!("[service.restarted] pd={} task={} restarts={} pages={} dropped_messages={} admitted={}", s.pd, id, count, s.count, dropped, admitted as u8);
    crate::crash_log::service(s.pd, count, false);
    // A new Shell starts from nothing: Display starts again too, so that it forgets
    // the old windows and announces itself ready to the new Shell.
    if s.pd == SHELL_PD { request_end(DISPLAY_PD); }
}
