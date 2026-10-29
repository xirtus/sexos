// SPDX-License-Identifier: GPL-3.0-only
//! Two immutable BOOT_DAG client watches. Hints confer no cleanup authority.
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU8, Ordering};
use crate::scheduler::{Task, task_ownership::Phase};
const LIVE:u64=5;
const RETRY:u64=2;
const DENIED:u64=3;
struct Watch {
    target:AtomicU32, pager:AtomicU32, task:AtomicPtr<Task>,
    kicked:AtomicBool, notice:AtomicU8, // 0 pending,1 publishing,2 accepted,3 cleanup acknowledged
}
impl Watch {
    const fn new()->Self{Self{target:AtomicU32::new(0),pager:AtomicU32::new(0),task:AtomicPtr::new(core::ptr::null_mut()),kicked:AtomicBool::new(false),notice:AtomicU8::new(0)}}
}
static WATCHES:[Watch;2]=[const{Watch::new()};2];
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct DeadTarget{pub task_id:u32}

pub fn register_paging_exit_watch(target:u32,pager:u32){
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"EXIT_WATCH_AFTER_BOOT");
    assert!(target!=0 && pager!=0 && target!=pager,"EXIT_WATCH_INVALID_ID");
    let pd=crate::ipc::DOMAIN_REGISTRY.get(target).expect("EXIT_WATCH_NO_TARGET");
    assert!(crate::ipc::DOMAIN_REGISTRY.get(pager).is_some(),"EXIT_WATCH_NO_PAGER");
    let task=pd.main_task.load(Ordering::Acquire);assert!(!task.is_null(),"EXIT_WATCH_NO_MAIN");
    assert!(!WATCHES.iter().any(|w|w.target.load(Ordering::Acquire)==target),"EXIT_WATCH_DUPLICATE");
    let watch=WATCHES.iter().find(|w|w.target.compare_exchange(0,u32::MAX,Ordering::AcqRel,Ordering::Acquire).is_ok()).expect("EXIT_WATCH_CAPACITY");
    watch.pager.store(pager,Ordering::Relaxed);watch.task.store(task,Ordering::Relaxed);watch.target.store(target,Ordering::Release);
}
fn find(target:u32,pager:u32)->Result<&'static Watch,u64>{
    WATCHES.iter().find(|w|w.target.load(Ordering::Acquire)==target && target!=0
        && w.pager.load(Ordering::Acquire)==pager).ok_or(DENIED)
}
fn validate(watch:&Watch)->Result<DeadTarget,u64>{
    let target=watch.target.load(Ordering::Acquire);
    let pd=crate::ipc::DOMAIN_REGISTRY.get(target).ok_or(DENIED)?;
    let task=pd.main_task.load(Ordering::Acquire);
    if task.is_null() || task!=watch.task.load(Ordering::Acquire){return Err(DENIED);}
    let state=unsafe{(*task).state.snapshot()};
    if !state.exited{return Err(LIVE);}
    if state.phase!=Phase::Dead || state.owner_core.is_some(){return Err(RETRY);}
    // A domain shares its demand VA. Its signal trampoline must never be active
    // when main-task cleanup detaches those pages. Boot trampolines are inert.
    let trampoline=pd.trampoline_task.load(Ordering::Acquire);
    if !trampoline.is_null(){
        let other=unsafe{(*trampoline).state.snapshot()};
        if !matches!(other.phase,Phase::Constructed|Phase::Dead) || other.owner_core.is_some(){return Err(RETRY);}
    }
    Ok(DeadTarget{task_id:unsafe{(*task).id}})
}
pub fn validate_dead_target(target:u32,pager:u32)->Result<DeadTarget,u64>{validate(find(target,pager)?)}
pub fn acknowledge_dead_cleanup(target:u32,pager:u32,task_id:u32)->bool{
    let Ok(watch)=find(target,pager)else{return false;};
    if validate(watch)!=Ok(DeadTarget{task_id}){return false;}
    watch.notice.store(3,Ordering::Release);true
}
/// Called with kernel authority and IF masked, including idle timer interrupts.
/// Exit intent first wakes the pager so a last-runnable exiting owner can switch
/// away. Only Dead (after stack release) publishes the authenticated kernel hint.
pub fn poll_paging_exit_notifications(){
    crate::app_resources::poll_exit_notifications();
    for watch in &WATCHES{
        let target=watch.target.load(Ordering::Acquire);
        if target==0 || target==u32::MAX || watch.notice.load(Ordering::Acquire)>=2{continue;}
        let Some(pd)=crate::ipc::DOMAIN_REGISTRY.get(target)else{continue;};
        let task=pd.main_task.load(Ordering::Acquire);
        if task.is_null() || task!=watch.task.load(Ordering::Acquire) || !unsafe{(*task).state.snapshot().exited}{continue;}
        let pager_id=watch.pager.load(Ordering::Acquire);
        let Some(pager)=crate::ipc::DOMAIN_REGISTRY.get(pager_id)else{continue;};
        let pager_task=pager.main_task.load(Ordering::Acquire);
        if !watch.kicked.swap(true,Ordering::AcqRel){crate::scheduler::unpark_thread(pager_task);}
        let Ok(dead)=validate(watch)else{continue;};
        if watch.notice.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire).is_err(){continue;}
        let message=crate::ipc::messages::MessageType::IpcCall{
            func_id:sex_pdx::paging::OP_TASK_EXIT,arg0:target as u64,arg1:dead.task_id as u64,arg2:0,caller_pd:0};
        let accepted=unsafe{(*pager.message_ring).enqueue(message)}.is_ok();
        let _=watch.notice.compare_exchange(1,if accepted{2}else{0},Ordering::AcqRel,Ordering::Acquire);
        if accepted{
            crate::scheduler::unpark_thread(pager_task);
            crate::serial_println!("[paging.exit.notice] target={} task={} pager={} detached=1 accepted=1",target,dead.task_id,pager_id);
        }
    }
}
