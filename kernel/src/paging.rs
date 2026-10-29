// SPDX-License-Identifier: GPL-3.0-only
//! Narrow sex-pdx-authorized leaf transition mechanism. Sext owns frame choice,
//! zeroing and policy. All backing/VA/page-table allocation happened at boot.
use crate::capability::{CapabilityData, ProtectionDomain};
use crate::paging_state::*;
use spin::Mutex;

struct PagingState {
    pool: FramePoolGrant,
    mappings: [PagerMappingGrant; CLIENTS],
    pager_pd: u32,
    target_pkeys: [u8; CLIENTS],
    physical: u64,
    hhdm: u64,
    leaf_addresses: [[u64; PAGES]; CLIENTS],
    ledger: MappingLedger,
}
static STATE: Mutex<Option<PagingState>> = Mutex::new(None);

pub(crate) fn initialize(pool: FramePoolGrant, mappings: [PagerMappingGrant; CLIENTS], pager_pd: u32,
    target_pkeys: [u8; CLIENTS], physical: u64, leaf_addresses: [[u64; PAGES]; CLIENTS])
{
    assert_eq!(crate::ipc::BOOT_PHASE.load(core::sync::atomic::Ordering::Acquire),
        crate::ipc::BootPhase::RegistryBuild as u8);
    let mut state = STATE.lock();
    assert!(state.is_none(), "PAGING_GENESIS_REPEATED");
    *state = Some(PagingState { pool, mappings, pager_pd, target_pkeys, physical,
        hhdm: crate::HHDM_REQUEST.response().unwrap().offset, leaf_addresses, ledger: MappingLedger::new() });
}

fn cap(pd: &ProtectionDomain, slot: u64) -> Option<CapabilityData> {
    let slot = crate::capability_slot::decode_capability_slot(slot)?;
    pd.find_capability(slot).map(|cap|cap.data)
}

fn mapping(pd: &ProtectionDomain, slot: u64) -> Result<PagerMappingGrant,u64> {
    match cap(pd,slot) { Some(CapabilityData::PagerMapping(map)) => Ok(map), _ => Err(INVALID) }
}

pub fn info(pd: &ProtectionDomain, slot: u64) -> [u64;5] {
    match cap(pd,slot) {
        Some(CapabilityData::FramePool(pool)) => [0,pool.base_va,pool.length,0,(READ|WRITE) as u64],
        Some(CapabilityData::PagerMapping(map)) => [0,map.base_va,map.length,map.target_pd as u64,map.rights as u64],
        Some(CapabilityData::DemandRegion(region)) => [0,region.base_va,region.length,pd.id as u64,region.rights as u64],
        _ => [INVALID,0,0,0,0],
    }
}

pub fn query(pd: &ProtectionDomain, slot: u64, token: u64) -> [u64;5] {
    let result = (|| {
        let map = mapping(pd,slot)?;
        {
            let guard = STATE.try_lock().ok_or(RETRY)?;
            let state = guard.as_ref().ok_or(INVALID)?;
            if !state.mappings.contains(&map) || pd.id != state.pager_pd
                || state.ledger.cleanup_record(map.range_id).is_some() { return Err(DENIED); }
        }
        let fault = crate::ipc::pagefault::query_fault(map.target_pd,pd.id,token)?;
        // Expose authenticated pending metadata even outside this map extent:
        // sext must be able to reject its client's unsatisfiable fault. Commit
        // still enforces exact reserved bounds; query creates no mapping right.
        Ok([0,fault.page_addr,fault.error_code as u64,map.target_pd as u64,map.rights as u64])
    })();
    result.unwrap_or_else(|error|[error,0,0,0,0])
}

pub fn reject(pd: &ProtectionDomain, slot: u64, token: u64) -> u64 {
    match mapping(pd,slot).and_then(|map|crate::ipc::pagefault::reject_fault(map.target_pd,pd.id,token)) {
        Ok(()) => 0, Err(error) => error,
    }
}

fn finish(binding: Binding) -> u64 {
    // Mapping ownership survives task exit or temporary completion contention.
    // Exact retries resume this step without re-zeroing or replacing the PTE.
    if crate::ipc::pagefault::complete_fault(binding.target_pd,binding.pager_pd,binding.token).is_err() {
        // A published frame remains owned until authorized finalized reclaim.
        // Every completion failure is
        // uncertain/retry, never a pre-publication rejection that could let sext
        // recycle or re-zero backing still visible to the client.
        return RETRY;
    }
    let mut state = STATE.lock();
    let state = state.as_mut().expect("PAGING_STATE_DISAPPEARED");
    state.ledger.finalize(binding).expect("PAGING_FINALIZE_IDENTITY");
    0
}

pub fn commit(pd: &ProtectionDomain, map_slot: u64, pool_slot: u64, frame: u64, token: u64, rights: u64) -> u64 {
    let map = match mapping(pd,map_slot) { Ok(map)=>map, Err(error)=>return error };
    let pool = match cap(pd,pool_slot) { Some(CapabilityData::FramePool(pool))=>pool, _=>return INVALID };
    let existing = {
        let Some(guard) = STATE.try_lock() else { return RETRY; };
        let Some(state) = guard.as_ref() else { return INVALID; };
        if !state.mappings.contains(&map) || pool != state.pool || pd.id != state.pager_pd { return DENIED; }
        if state.ledger.cleanup_record(map.range_id).is_some() { return DENIED; }
        let record = state.ledger.token(token);
        if let Some(record) = record {
            let binding = record.binding;
            if binding.frame_index as u64 != frame || binding.rights as u64 != rights
                || binding.target_pd != map.target_pd || binding.pager_pd != pd.id || binding.range_id != map.range_id
            { return DENIED; }
        }
        if record.is_none() && state.ledger.is_retired(map.range_id,token) { return DENIED; }
        record
    };
    if let Some(record) = existing {
        return if record.finalized { 0 } else { finish(record.binding) };
    }

    let fault = match crate::ipc::pagefault::claim_fault(map.target_pd,pd.id,token) {
        Ok(fault)=>fault, Err(error)=>return error,
    };
    let binding = validate(map,pool,pd.id,FaultView {task_id:fault.task_id,token:fault.token,target_pd:fault.target_pd,
        pager_pd:fault.pager_pd,page:fault.page_addr,error:fault.error_code as u64},frame,rights);
    let prepared = (|| {
        let binding = binding?;
        let Some(mut guard) = STATE.try_lock() else { return Err(RETRY); };
        let state = guard.as_mut().ok_or(INVALID)?;
        state.ledger.check_new(binding,map.quota)?;
        let range = binding.range_id as usize - 1;
        let leaf = state.leaf_addresses[range][binding.page_index] as *mut u64;
        if unsafe { core::ptr::read_volatile(leaf) } != 0 { return Err(DENIED); }
        let physical = state.physical + frame * PAGE;
        // Sext must zero its selected frame before exposure. Verification here
        // changes no contents and confers no frame-selection policy.
        let contents = (state.hhdm + physical) as *const u64;
        for word in 0..512 {
            if unsafe { core::ptr::read_volatile(contents.add(word)) } != 0 { return Err(DENIED); }
        }
        let flags = 1u64 | 4 | (1u64 << 63) | ((state.target_pkeys[range] as u64) << 59)
            | if binding.rights & WRITE != 0 { 2 } else { 0 };
        state.ledger.install(binding,map.quota)?;
        unsafe {
            core::ptr::write_volatile(leaf,physical | flags);
            x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(fault.page_addr));
        }
        crate::serial_println!("[paging.leaf.commit] token={} target={} page={:#x} frame={} rights={} pkey={} zero=1 runtime_alloc=0",
            token,map.target_pd,fault.page_addr,frame,rights,state.target_pkeys[range]);
        Ok(binding)
    })();
    match prepared {
        Ok(binding) => finish(binding),
        Err(error) => {
            assert!(crate::ipc::pagefault::rollback_fault(map.target_pd,pd.id,token),"PAGING_CLAIM_ROLLBACK_FAILED");
            error
        }
    }
}

/// BSP-only transition: syscall entry masks IF and APs remain parked. This is
/// local invalidation, not an SMP shootdown protocol. A successful detach is
/// irreversible; no fallible operation follows the clear/flush boundary.
pub fn reclaim(pd: &ProtectionDomain, map_slot: u64, pool_slot: u64, frame: u64, token: u64) -> u64 {
    let map = match mapping(pd,map_slot) { Ok(map)=>map, Err(error)=>return error };
    let pool = match cap(pd,pool_slot) { Some(CapabilityData::FramePool(pool))=>pool, _=>return INVALID };
    if frame >= PAGES as u64 || token == 0 { return DENIED; }
    let Some(mut guard) = STATE.try_lock() else { return RETRY; };
    let Some(state) = guard.as_mut() else { return INVALID; };
    if !state.mappings.contains(&map) || pool != state.pool || pd.id != state.pager_pd { return DENIED; }
    let exact = |b: Binding| b.token == token && b.target_pd == map.target_pd
        && b.pager_pd == pd.id && b.range_id == map.range_id && b.frame_index as u64 == frame;
    // Keep the last retired tuple even during subsequent occupancy. Its exact
    // replay succeeds without reading or changing the new owner's leaf. Once
    // replaced by a newer retirement, older requests fail closed.
    if state.ledger.retired(frame as usize).is_some_and(exact) { return 0; }
    let Some(record) = state.ledger.entries[frame as usize] else { return DENIED; };
    if !exact(record.binding) || !record.finalized { return DENIED; }
    let binding = record.binding;
    let range = binding.range_id as usize - 1;
    let leaf = state.leaf_addresses[range][binding.page_index] as *mut u64;
    let expected = (state.physical + frame * PAGE) | 1 | 4 | (1u64 << 63)
        | ((state.target_pkeys[range] as u64) << 59) | if binding.rights & WRITE != 0 { 2 } else { 0 };
    // Only processor-owned Accessed/Dirty bits may differ from the installed
    // exact physical/permissions/pkey identity.
    if unsafe { core::ptr::read_volatile(leaf) } & !0x60 != expected { return DENIED; }
    let page = map.base_va + binding.page_index as u64 * PAGE;
    unsafe {
        core::ptr::write_volatile(leaf,0);
        x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(page));
    }
    state.ledger.retire(binding).expect("PAGING_RETIRE_IDENTITY");
    crate::serial_println!("[paging.leaf.reclaim] token={} target={} page={:#x} frame={} cleared=1 tlb=1 runtime_alloc=0",
        token,map.target_pd,page,frame);
    0
}

/// Tear down one permanently exited boot target. Sext chooses when to invoke
/// this capability operation; no allocator policy or new authority is created.
/// All preconditions are checked before any leaf changes. BSP/parked-AP scope
/// matches reclaim: all leaves are flushed before any frame becomes reusable.
pub fn cleanup(pd: &ProtectionDomain, map_slot: u64, pool_slot: u64) -> [u64;3] {
    let result = (|| {
        let map = mapping(pd,map_slot)?;
        let pool = match cap(pd,pool_slot) { Some(CapabilityData::FramePool(pool))=>pool, _=>return Err(INVALID) };
        let mut guard = STATE.try_lock().ok_or(RETRY)?;
        let state = guard.as_mut().ok_or(INVALID)?;
        if !state.mappings.contains(&map) || pool != state.pool || pd.id != state.pager_pd { return Err(DENIED); }
        if let Some(done) = state.ledger.cleanup_record(map.range_id) {
            return Ok([0,done.frames as u64,done.task_id as u64]);
        }
        let dead = crate::scheduler::validate_dead_target(map.target_pd,pd.id)?;
        let plan = state.ledger.cleanup_plan(map.range_id,map.target_pd,pd.id,dead.task_id)?;
        let range = map.range_id as usize - 1;
        // Preflight every occupied leaf before changing any of them. Requiring
        // empty unowned leaves also prevents successful closure with an orphan.
        for page in 0..PAGES {
            let binding = plan.iter().flatten().find(|b|b.page_index == page);
            let actual = unsafe { core::ptr::read_volatile(state.leaf_addresses[range][page] as *const u64) };
            let Some(binding) = binding else {
                if actual != 0 { return Err(DENIED); }
                continue;
            };
            let expected = (state.physical + binding.frame_index as u64 * PAGE) | 1 | 4 | (1u64 << 63)
                | ((state.target_pkeys[range] as u64) << 59) | if binding.rights & WRITE != 0 { 2 } else { 0 };
            if actual & !0x60 != expected { return Err(DENIED); }
        }
        // This can still fail/contend, so perform it before the first PTE write.
        // It accepts a published but unfinished Claimed fault of this exact task.
        crate::ipc::pagefault::retire_dead_fault(map.target_pd,pd.id,dead.task_id)?;
        let mut count = 0;
        for binding in plan.into_iter().flatten() {
            let page = map.base_va + binding.page_index as u64 * PAGE;
            unsafe {
                core::ptr::write_volatile(state.leaf_addresses[range][binding.page_index] as *mut u64,0);
                x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(page));
            }
            count += 1;
            crate::serial_println!("[paging.leaf.cleanup] target={} task={} token={} page={:#x} frame={} finalized={} cleared=1 tlb=1 runtime_alloc=0",
                map.target_pd,dead.task_id,binding.token,page,binding.frame_index,
                state.ledger.entries[binding.frame_index].unwrap().finalized as u8);
        }
        let done = state.ledger.close_exited_range(map.range_id,map.target_pd,pd.id,dead.task_id)
            .expect("PAGING_CLEANUP_IDENTITY");
        assert!(crate::scheduler::acknowledge_dead_cleanup(map.target_pd,pd.id,dead.task_id),"PAGING_CLEANUP_ACK_IDENTITY");
        crate::serial_println!("[paging.target.cleanup] target={} task={} frames={:#x} leaves={} closed=1 detached=1 all_flushed=1 runtime_alloc=0",
            map.target_pd,dead.task_id,done.frames,count);
        Ok([0,done.frames as u64,done.task_id as u64])
    })();
    result.unwrap_or_else(|error|[error,0,0])
}

/// Private-image retirement must wait for every demand range of this boot app.
/// Apps with no demand grant have no paging cleanup obligation.
pub fn target_cleanup_complete(target:u32)->Result<bool,u64>{
    let guard=STATE.try_lock().ok_or(RETRY)?;
    let state=guard.as_ref().ok_or(INVALID)?;
    Ok(state.mappings.iter().filter(|map|map.target_pd==target)
        .all(|map|state.ledger.cleanup_record(map.range_id).is_some()))
}
