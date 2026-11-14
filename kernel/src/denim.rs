// SPDX-License-Identifier: GPL-3.0-only
//! Compressed memory, the kernel's half (`sex_pdx::denim`): Denim may take a cold heap
//! page of a live program run and give it back on the run's next touch. The kernel
//! moves bytes and leaves; what to take, when, and how to store it is Denim's.
//!
//! Denim names a window: a page of its own grown memory, found here by its leaf, so
//! the kernel copies through the direct map and Denim never maps a program's page.
use sex_pdx::denim as d;
use core::sync::atomic::Ordering;

/// The frame behind a present, writable page of Denim's own memory at `va`.
fn window(va: u64) -> Result<u64, u64> {
    let denim = crate::ipc::DOMAIN_REGISTRY.get(d::DENIM_PD).ok_or(d::DENIED)?;
    if va & 0xFFF != 0 { return Err(d::INVALID); }
    let leaf = crate::boot_memory::existing_leaf_address(va).ok_or(d::DENIED)?;
    let v = unsafe { core::ptr::read_volatile(leaf as *const u64) };
    if v & 7 != 7 || (v >> 59) & 15 != denim.pku_key as u64 { return Err(d::DENIED); }
    Ok(v & crate::app_resource_state::PHYSICAL_MASK)
}

/// Whether `va` is a page Denim holds for `target`: its leaf is the row's marker.
pub fn holds(target: u32, va: u64) -> bool {
    crate::ipc::DOMAIN_REGISTRY.get(d::DENIM_PD).is_some()
        && crate::boot_memory::existing_leaf_address(va & !0xFFF)
            .is_some_and(|leaf| unsafe { core::ptr::read_volatile(leaf as *const u64) } == d::out_marker(target))
}

fn post(func_id: u64, target: u32, page: u64, token: u64) -> Result<(), u64> {
    let denim = crate::ipc::DOMAIN_REGISTRY.get(d::DENIM_PD).ok_or(d::DENIED)?;
    let msg = crate::ipc::messages::MessageType::IpcCall { func_id, arg0: target as u64, arg1: page, arg2: token, caller_pd: 0, caller_task: 0 };
    unsafe { (*denim.message_ring).enqueue(msg) }.map_err(|_| d::RETRY)?;
    crate::scheduler::unpark_thread(denim.main_task.load(Ordering::Acquire));
    Ok(())
}
/// A run's main task touched a page Denim holds; it is parked under `token`.
pub fn notify(target: u32, page: u64, token: u64) -> Result<(), u64> { post(d::OP_FAULT, target, page, token) }
/// The run needs every page back (a worker, or code) before it may go on.
pub fn recall(target: u32) { let _ = post(d::OP_RECALL, target, 0, 0); }

/// SYS 88, Denim only.
pub fn call(caller: u32, op: u64, target: u64, a: u64, b: u64, c: u64) -> [u64; 5] {
    if caller != d::DENIM_PD { return [d::DENIED, 0, 0, 0, 0]; }
    let Ok(target) = u32::try_from(target) else { return [d::INVALID, 0, 0, 0, 0] };
    let two = |r: [u64; 2]| [r[0], r[1], 0, 0, 0];
    match op {
        d::SCAN => crate::app_resources::denim_scan(target, a),
        d::OUT => match window(b) { Ok(w) => two(crate::app_resources::denim_out(target, a, w)), Err(e) => [e, 0, 0, 0, 0] },
        d::IN => match window(b) { Ok(w) => two(page_in(target, a, w, c)), Err(e) => [e, 0, 0, 0, 0] },
        _ => [d::INVALID, 0, 0, 0, 0],
    }
}

/// With a token, the parked task goes on once its page is back, or once the page can
/// never come back (its run was ended): it then faults again and ends as any fault does.
fn page_in(target: u32, va: u64, window: u64, token: u64) -> [u64; 2] {
    use crate::ipc::pagefault as f;
    if token == 0 { return crate::app_resources::denim_in(target, va, window); }
    let fault = match f::claim_fault(target, d::DENIM_PD, token) { Ok(r) => r, Err(e) => return [e, 0] };
    if fault.page_addr != va & !0xFFF { f::rollback_fault(target, d::DENIM_PD, token); return [d::DENIED, 0]; }
    let reply = crate::app_resources::denim_in(target, va, window);
    let gone = reply[0] == d::DENIED && !crate::boot_memory::existing_leaf_address(va)
        .is_some_and(|leaf| unsafe { core::ptr::read_volatile(leaf as *const u64) } == d::out_marker(target));
    if reply[0] != d::OK && !gone { f::rollback_fault(target, d::DENIM_PD, token); return reply; }
    match f::complete_fault(target, d::DENIM_PD, token) { Ok(()) => reply, Err(e) => [e, 0] }
}
