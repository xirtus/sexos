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
const SERVICES: usize = 4;
static SERVICE: [Mutex<Service>; SERVICES] = [const { Mutex::new(Service { pd: 0, entry: 0, stack_top: 0, pages: [Page { frame: 0, pristine: 0 }; MAX_PAGES], count: 0 }) }; SERVICES];
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
    let mut s = SERVICE.iter().map(|m| m.lock()).find(|s| s.pd == 0).expect("SERVICE_RESTART_SLOTS");
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
    crate::serial_println!("[boot.service.restartable] pd={} pages={} entry={:#x}", pd_id, s.count, s.entry);
}

/// How many times the service in domain `pd` has been started again.
pub fn restarts_of(pd: u32) -> u32 {
    (0..SERVICES).find(|&i| SERVICE[i].try_lock().is_some_and(|s| s.pd == pd && pd != 0))
        .map_or(0, |i| RESTARTS[i].load(Ordering::Relaxed))
}

/// Display and Shell hold the whole desktop's state and cannot yet start again in place.
/// If either ends, the machine stops (screen, record, restart) instead of freezing.
const DESKTOP: [u32; 2] = [1, 3];
static DESKTOP_STOPPED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// From the timer, IF masked: a service whose task has ended starts again.
pub fn poll() {
    for index in 0..SERVICES { poll_one(index); }
    // New crash records (a fault, this restart) go to Files for the disk ring.
    crate::crash_log::poll();
    for pd_id in DESKTOP {
        let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(pd_id) else { continue };
        let main = pd.main_task.load(Ordering::Acquire);
        if main.is_null() { continue; }
        let state = unsafe { (*main).state.snapshot() };
        if state.exited && state.phase == crate::scheduler::task_ownership::Phase::Dead
            && !DESKTOP_STOPPED.swap(true, Ordering::AcqRel) {
            crate::console::stop(format_args!("{} ended. The desktop cannot go on without it, so the machine restarts.",
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
    for page in &s.pages[..s.count] { crate::boot_memory::copy_frame(page.pristine, page.frame); }
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
}
