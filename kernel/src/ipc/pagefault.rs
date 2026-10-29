use crate::ipc::safe_pdx_call;
use crate::scheduler::{Task, SCHEDULERS};
use crate::scheduler::fault_transaction::{TokenIssuer, INVALID, RETRY, DENIED};
pub use crate::scheduler::fault_transaction::FaultRecord;
use core::sync::atomic::Ordering;
use sex_pdx::paging::{FaultNotice, OP_FAULT_NOTICE};

static TOKENS: TokenIssuer = TokenIssuer::new();

fn target_task(target_pd: u32) -> Result<*mut Task, u64> {
    if target_pd == 0 { return Err(INVALID); }
    let pd = crate::ipc::DOMAIN_REGISTRY.get(target_pd).ok_or(INVALID)?;
    let task = pd.main_task.load(Ordering::Acquire);
    if task.is_null() { Err(INVALID) } else { Ok(task) }
}

fn is_live_fault(task: *mut Task) -> bool {
    let state = unsafe { (*task).state.snapshot() };
    state.fault_blocked && !state.exited
}

fn validate_record(task: *mut Task, target_pd: u32, record: FaultRecord) -> Result<FaultRecord, u64> {
    if record.target_pd != target_pd || record.task_id != unsafe { (*task).id } { return Err(INVALID); }
    Ok(record)
}

/// The caller must save the complete user frame and establish fault-specific
/// ownership before publication. No user-provided PD ID or context pointer is
/// accepted by the pager completion ABI.
pub unsafe fn forward_page_fault(fault_addr: u64, error_code: u64, pd_id: u64) -> Result<u64, u64> {
    let core = crate::core_local::CoreLocal::get();
    let pd = core.current_pd_ref();
    if pd_id != pd.id as u64 { return Err(INVALID); }
    let task = target_task(pd.id)?;
    if SCHEDULERS[core.core_id as usize].current_task.load(Ordering::Acquire) != task
        || !is_live_fault(task) { return Err(DENIED); }
    let cap = pd.find_capability(sex_pdx::SLOT_SEXT as u32).ok_or(DENIED)?;
    let pager = match cap.data {
        crate::capability::CapabilityData::Domain(target)
            if target != 0 && target != pd.id && crate::ipc::DOMAIN_REGISTRY.get(target).is_some() => target,
        _ => return Err(DENIED),
    };
    let token = TOKENS.issue()?;
    let notice = FaultNotice::decode(OP_FAULT_NOTICE, pd.id as u64, [fault_addr, error_code, token])
        .map_err(|_| INVALID)?;
    let record = FaultRecord { token, target_pd: pd.id, pager_pd: pager, task_id: (*task).id,
        fault_addr, page_addr: notice.page(), error_code,
        saved_rip: (*task).context.rip, saved_frame: (*task).context.kstack_top };
    // RETRY here means lock contention only. An existing active request is a
    // distinct invariant failure, never an unbounded wait on our own request.
    loop {
        match (*task).fault.prepare(record) {
            Err(RETRY) => core::hint::spin_loop(),
            result => { result?; break; }
        }
    }
    let [arg0, arg1, arg2] = notice.args();
    if let Err(error) = safe_pdx_call(sex_pdx::SLOT_SEXT as u32, OP_FAULT_NOTICE, arg0, arg1, arg2) {
        loop {
            match (*task).fault.cancel_prepared(token) {
                Err(RETRY) => core::hint::spin_loop(),
                result => { result?; break; }
            }
        }
        return Err(error);
    }
    // A receiver on a future concurrent lane may observe the notice first; its
    // query gets RETRY until the accepted publication is finalized here.
    loop {
        match (*task).fault.publish(token) {
            Err(RETRY) => core::hint::spin_loop(),
            result => { result?; break; }
        }
    }
    crate::serial_println!("[paging.fault.pending] target={} task={} token={} page={:#x} rip={:#x}",
        pd.id, (*task).id, token, record.page_addr, record.saved_rip);
    Ok(token)
}

pub fn query_fault(target_pd: u32, pager_pd: u32, token: u64) -> Result<FaultRecord, u64> {
    let task = target_task(target_pd)?;
    if !is_live_fault(task) { return Err(DENIED); }
    validate_record(task, target_pd, unsafe { (*task).fault.query(pager_pd, token)? })
}

pub fn claim_fault(target_pd: u32, pager_pd: u32, token: u64) -> Result<FaultRecord, u64> {
    let task = target_task(target_pd)?;
    if !is_live_fault(task) { return Err(DENIED); }
    let record = unsafe { (*task).fault.claim(pager_pd, token)? };
    let checked = validate_record(task, target_pd, record);
    if checked.is_err() || !is_live_fault(task) {
        rollback_fault(target_pd, pager_pd, token);
        return Err(DENIED);
    }
    checked
}

/// Caller guarantees that no PTE was changed and no selected frame became live.
pub fn rollback_fault(target_pd: u32, pager_pd: u32, token: u64) -> bool {
    let Ok(task) = target_task(target_pd) else { return false; };
    // The claim belongs to this kernel operation. Retry only lock contention;
    // never strand a claimed transaction after a no-effect commit rejection.
    loop {
        match unsafe { (*task).fault.rollback(pager_pd, token) } {
            Err(RETRY) => core::hint::spin_loop(),
            result => return result.is_ok(),
        }
    }
}

/// Called only after capability validation, exact leaf installation and TLB
/// invalidation. Mapping remains committed even if its task has since exited.
/// Retire all mutable fault metadata BEFORE exposing runnable ownership.
pub fn complete_fault(target_pd: u32, pager_pd: u32, token: u64) -> Result<(), u64> {
    let task = target_task(target_pd)?;
    let Some(record) = (unsafe { (*task).fault.complete(pager_pd, token)? }) else { return Ok(()); };
    assert!(validate_record(task, target_pd, record).is_ok(), "COMMITTED_FAULT_TASK_IDENTITY_CHANGED");
    let resumed = unsafe { crate::scheduler::resume_resolved_fault(task) };
    crate::serial_println!("[paging.fault.completed] target={} task={} token={} resume={} saved_rip={:#x}",
        target_pd, record.task_id, token, resumed as u8, record.saved_rip);
    Ok(())
}

pub fn reject_fault(target_pd: u32, pager_pd: u32, token: u64) -> Result<(), u64> {
    let task = target_task(target_pd)?;
    let record = unsafe { (*task).fault.reject(pager_pd, token)? };
    validate_record(task, target_pd, record)?;
    unsafe { (*task).state.exit(); }
    crate::serial_println!("[paging.fault.rejected] target={} task={} token={} terminal=1", target_pd, record.task_id, token);
    Ok(())
}

/// Called after bounded paging cleanup preflights every published leaf, before
/// any leaf is cleared. Lock contention has no effect and is safely retried.
pub fn retire_dead_fault(target_pd: u32, pager_pd: u32, task_id: u32) -> Result<(), u64> {
    let dead = crate::scheduler::validate_dead_target(target_pd, pager_pd)?;
    if dead.task_id != task_id { return Err(DENIED); }
    let task = target_task(target_pd)?;
    unsafe { (*task).fault.retire_exited(target_pd, pager_pd, task_id) }
}
