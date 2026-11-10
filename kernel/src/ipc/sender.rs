// SPDX-License-Identifier: GPL-3.0-only
//! Capture transport provenance from the executing kernel scheduler context.
//! Task IDs are not capability handles. Service-specific held authority and
//! lifecycle checks remain required after this authenticated transport stamp.
use core::sync::atomic::Ordering;

/// No active task, an inconsistent PD binding, terminal task or zero task ID
/// cannot supply application sender provenance. Kernel-generated messages use
/// the explicit reserved (PD0, task0) source instead of inventing a task.
pub fn current_task(caller_pd: u32) -> Option<u32> {
    let core = crate::core_local::CoreLocal::get();
    let pd = core.current_pd_ref();
    if core.current_pd() != caller_pd || pd.id != caller_pd { return None; }
    let scheduler = crate::scheduler::SCHEDULERS.get(core.core_id as usize)?;
    let pointer = scheduler.current_task.load(Ordering::Acquire);
    let task = unsafe { pointer.as_ref()? };
    if task.id == 0 || task.context.pd_id != caller_pd as u64
        || task.context.pd_ptr != pd as *const _ || task.state.snapshot().exited {
        return None;
    }
    // A program's worker shares its run's domain and authority, so it speaks as the
    // run: services that bound the run's main task answer its workers too.
    let main = pd.main_task.load(Ordering::Acquire);
    if !main.is_null() && main != pointer && crate::program_workers::is_worker(pointer) {
        let main = unsafe { &*main };
        if main.id != 0 && !main.state.snapshot().exited { return Some(main.id); }
    }
    Some(task.id)
}
