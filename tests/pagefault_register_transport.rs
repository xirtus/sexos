//! Actual production forwarder, codecs, transaction and ownership with only the
//! registry/dispatch/context boundary doubled. No PTE or x86 execution claim.
extern crate self as sex_pdx;
#[path = "../kernel/src/ipc/pagefault.rs"] mod kernel_pagefault;
#[path = "../crates/sex-pdx/src/pagefault.rs"] pub mod pagefault;
#[path = "../crates/sex-pdx/src/paging.rs"] pub mod paging;
#[path = "../kernel/src/task_ownership.rs"] pub mod ownership;
#[path = "../kernel/src/fault_transaction.rs"] pub mod fault_transaction;
pub const SLOT_SEXT:u64=2;
#[macro_export] macro_rules! serial_println { ($($arg:tt)*) => { let _ = format_args!($($arg)*); }; }
use pagefault::{is_canonical_fault_address,PageFaultError,PageFaultRequest,OP_PAGE_FAULT};
use std::{cell::RefCell,sync::{Mutex,atomic::{AtomicPtr,AtomicUsize,Ordering}}};
use fault_transaction::{INVALID,DENIED,RETRY};
static SERIAL:Mutex<()>=Mutex::new(());
static RESUMES:AtomicUsize=AtomicUsize::new(0);
#[derive(Clone,Copy,Debug,PartialEq,Eq)] struct Invocation{slot:u32,opcode:u64,args:[u64;3]}
struct Route {capability:Option<capability::Capability>,result:Result<u64,u64>,invocations:Vec<Invocation>,early:bool}
thread_local! {static ROUTE:RefCell<Route>=RefCell::new(Route{capability:None,result:Ok(0),invocations:vec![],early:false});}
mod capability {
 use super::*;
 #[derive(Clone,Copy)] pub enum CapabilityData{Domain(u32),IPC}
 #[derive(Clone,Copy)] pub struct Capability{pub data:CapabilityData}
 pub struct ProtectionDomain{pub id:u32,pub main_task:AtomicPtr<scheduler::Task>}
 impl ProtectionDomain{pub fn find_capability(&self,slot:u32)->Option<Capability>{assert_eq!(slot,2);ROUTE.with(|r|r.borrow().capability)}}
}
static PD:capability::ProtectionDomain=capability::ProtectionDomain{id:7,main_task:AtomicPtr::new(std::ptr::null_mut())};
static PAGER:capability::ProtectionDomain=capability::ProtectionDomain{id:9,main_task:AtomicPtr::new(std::ptr::null_mut())};
mod core_local {
 pub struct CoreLocal{pub core_id:u32} static CORE:CoreLocal=CoreLocal{core_id:0};
 impl CoreLocal{pub fn get()->&'static Self{&CORE} pub fn current_pd_ref(&self)->&'static super::capability::ProtectionDomain{&super::PD}}
}
mod scheduler {
 pub use super::fault_transaction;
 pub struct Context{pub rip:u64,pub kstack_top:u64}
 pub struct Task{pub id:u32,pub context:Context,pub state:super::ownership::TaskOwnership,pub fault:fault_transaction::FaultSlot}
 pub struct Scheduler{pub current_task:super::AtomicPtr<Task>}
 pub static SCHEDULERS:[Scheduler;1]=[Scheduler{current_task:super::AtomicPtr::new(std::ptr::null_mut())}];
 pub struct DeadTarget { pub task_id:u32 }
 pub fn validate_dead_target(target:u32,pager:u32)->Result<DeadTarget,u64>{
  if target!=7 || pager!=9{return Err(super::DENIED);}
  let task=super::PD.main_task.load(super::Ordering::Acquire);if task.is_null(){return Err(super::DENIED);}
  let state=unsafe{(*task).state.snapshot()};if !state.exited{return Err(5);}if state.phase!=super::ownership::Phase::Dead{return Err(super::RETRY);}
  Ok(DeadTarget{task_id:unsafe{(*task).id}})
 }
 pub unsafe fn resume_resolved_fault(task:*mut Task)->bool{let result=(*task).state.complete_fault();if result{super::RESUMES.fetch_add(1,super::Ordering::Relaxed);}result}
}
mod ipc {
 pub struct Registry;pub static DOMAIN_REGISTRY:Registry=Registry;
 impl Registry{pub fn get(&self,id:u32)->Option<&'static super::capability::ProtectionDomain>{match id{7=>Some(&super::PD),9=>Some(&super::PAGER),_=>None}}}
 pub fn safe_pdx_call(slot:u32,opcode:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{
  super::ROUTE.with(|r|{let mut r=r.borrow_mut();r.invocations.push(super::Invocation{slot,opcode,args:[a,b,c]});
   if r.early{assert_eq!(super::kernel_pagefault::query_fault(7,9,c),Err(super::RETRY));}
   r.result})
 }
}
fn setup()->*mut scheduler::Task{
 let state=ownership::TaskOwnership::new();assert!(state.activate());
 assert_eq!(state.try_admit(||Ok::<(),()>(())),ownership::Admission::Published);
 assert!(state.begin_dispatch(0));assert!(state.attach(0));assert_eq!(state.park_fault(0),ownership::ParkResult::Blocked);
 let task=Box::into_raw(Box::new(scheduler::Task{id:0x8000_0007,context:scheduler::Context{rip:0x100234,kstack_top:0x200800},state,fault:fault_transaction::FaultSlot::new()}));
 PD.main_task.store(task,Ordering::Release);scheduler::SCHEDULERS[0].current_task.store(task,Ordering::Release);RESUMES.store(0,Ordering::Relaxed);
 ROUTE.with(|r|*r.borrow_mut()=Route{capability:Some(capability::Capability{data:capability::CapabilityData::Domain(9)}),result:Ok(0),invocations:vec![],early:false});task
}
fn forward(a:u64,e:u64,p:u64)->Result<u64,u64>{unsafe{kernel_pagefault::forward_page_fault(a,e,p)}}
#[test]
fn round_trip_canonical_boundaries_and_protection_flags_by_value() {
    assert_ne!(OP_PAGE_FAULT, 0, "zero is the listen empty sentinel");
    for addr in [
        0,
        0x1000,
        0x0000_7fff_ffff_ffff,
        0xffff_8000_0000_0000,
        u64::MAX,
    ] {
        for code in [0, 1, 0x7, 0x27, u32::MAX] {
            for pd in [1, 7, u32::MAX] {
                let request = PageFaultRequest::new(addr, code, pd as u64).unwrap();
                assert_eq!(request.args(), [addr, code as u64, pd as u64]);
                assert_eq!(
                    PageFaultRequest::decode(OP_PAGE_FAULT, pd as u64, request.args()),
                    Ok(request)
                );
                assert_eq!(
                    (request.fault_addr(), request.error_code(), request.pd_id()),
                    (addr, code, pd)
                );
            }
        }
    }
}

#[test]
fn rejects_noncanonical_hole_without_confusing_high_canonical_reports() {
    for addr in [
        0x0000_8000_0000_0000,
        0x0000_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        0xffff_7fff_ffff_ffff,
    ] {
        assert!(!is_canonical_fault_address(addr));
        assert_eq!(
            PageFaultRequest::new(addr, 0, 7),
            Err(PageFaultError::NonCanonicalAddress)
        );
        assert_eq!(
            PageFaultRequest::decode(OP_PAGE_FAULT, 7, [addr, 0, 7]),
            Err(PageFaultError::NonCanonicalAddress)
        );
    }
}

#[test]
fn rejects_empty_wrong_opcode_truncation_and_forged_identity() {
    for opcode in [0, 1, OP_PAGE_FAULT + 1, u64::MAX] {
        assert_eq!(
            PageFaultRequest::decode(opcode, 7, [0x1000, 0, 7]),
            Err(PageFaultError::WrongOpcode)
        );
    }
    for pd in [0, u32::MAX as u64 + 1, u64::MAX] {
        assert_eq!(
            PageFaultRequest::new(0x1000, 0, pd),
            Err(PageFaultError::InvalidDomain)
        );
        assert_eq!(
            PageFaultRequest::decode(OP_PAGE_FAULT, pd, [0x1000, 0, pd]),
            Err(PageFaultError::InvalidDomain)
        );
    }
    assert_eq!(
        PageFaultRequest::decode(OP_PAGE_FAULT, 7, [0x1000, 1u64 << 32, 7]),
        Err(PageFaultError::ErrorCodeOverflow)
    );
    for caller in [0, 8, (1u64 << 32) + 7] {
        assert_eq!(
            PageFaultRequest::decode(OP_PAGE_FAULT, caller, [0x1000, 0, 7]),
            Err(PageFaultError::CallerMismatch)
        );
    }
}


#[test] fn live_transport_copies_token_and_binds_divergent_task_identity(){
 let _lock=SERIAL.lock().unwrap();let task=setup();ROUTE.with(|r|r.borrow_mut().early=true);
 let token=forward(0xdead_beef,6,7).unwrap();
 ROUTE.with(|r|{let r=r.borrow();assert_eq!(r.invocations,[Invocation{slot:2,opcode:paging::OP_FAULT_NOTICE,args:[0xdead_beef,6,token]}]);assert_eq!(paging::FaultNotice::decode(r.invocations[0].opcode,7,r.invocations[0].args).unwrap().caller_pd,7);});
 let record=kernel_pagefault::query_fault(7,9,token).unwrap();assert_eq!(record.task_id,unsafe{(*task).id});assert_ne!(record.task_id,record.target_pd);assert_eq!((record.saved_rip,record.saved_frame),(0x100234,0x200800));
}
#[test] fn malformed_protection_privilege_execute_and_null_never_enqueue(){
 let _lock=SERIAL.lock().unwrap();setup();
 for e in [0,1,2,3,5,7,4|8,4|16,4|32,4|64,u64::MAX]{assert_eq!(forward(0x1000,e,7),Err(INVALID));}
 for a in [0,1<<47,u64::MAX]{assert_eq!(forward(a,4,7),Err(INVALID));}
 for pd in [0,8,0x80000007,1<<32]{assert_eq!(forward(0x1000,4,pd),Err(INVALID));}
 ROUTE.with(|r|assert!(r.borrow().invocations.is_empty()));
}
#[test] fn nonmain_task_is_rejected_before_publication(){
 let _lock=SERIAL.lock().unwrap();setup();scheduler::SCHEDULERS[0].current_task.store(std::ptr::null_mut(),Ordering::Release);
 assert_eq!(forward(0x1000,4,7),Err(DENIED));ROUTE.with(|r|assert!(r.borrow().invocations.is_empty()));
}
#[test] fn invalid_pager_routes_never_enqueue(){
 let _lock=SERIAL.lock().unwrap();setup();
 for cap in [None,Some(capability::Capability{data:capability::CapabilityData::IPC}),Some(capability::Capability{data:capability::CapabilityData::Domain(0)}),Some(capability::Capability{data:capability::CapabilityData::Domain(7)}),Some(capability::Capability{data:capability::CapabilityData::Domain(8)})]{ROUTE.with(|r|r.borrow_mut().capability=cap);assert_eq!(forward(0x1000,4,7),Err(DENIED));}
 ROUTE.with(|r|assert!(r.borrow().invocations.is_empty()));
}
#[test] fn queue_failure_cancels_prepared_transaction_and_never_resumes(){
 let _lock=SERIAL.lock().unwrap();let task=setup();ROUTE.with(|r|r.borrow_mut().result=Err(99));assert_eq!(forward(0x1000,4,7),Err(99));
 let old=ROUTE.with(|r|r.borrow().invocations[0].args[2]);assert_eq!(kernel_pagefault::query_fault(7,9,old),Err(INVALID));
 assert!(unsafe{(*task).state.snapshot().fault_blocked});assert_eq!(RESUMES.load(Ordering::Relaxed),0);
 ROUTE.with(|r|r.borrow_mut().result=Ok(0));let next=forward(0x1000,4,7).unwrap();assert!(next>old);
}
#[test] fn foreign_stale_and_duplicate_completions_never_resume_wrong_task(){
 let _lock=SERIAL.lock().unwrap();let task=setup();let token=forward(0x1000,6,7).unwrap();
 assert_eq!(kernel_pagefault::claim_fault(7,8,token),Err(DENIED));assert_eq!(kernel_pagefault::claim_fault(7,9,token+1),Err(INVALID));assert!(kernel_pagefault::claim_fault(9,9,token).is_err());
 kernel_pagefault::claim_fault(7,9,token).unwrap();assert!(kernel_pagefault::rollback_fault(7,9,token));kernel_pagefault::claim_fault(7,9,token).unwrap();
 assert_eq!(kernel_pagefault::complete_fault(7,8,token),Err(DENIED));kernel_pagefault::complete_fault(7,9,token).unwrap();kernel_pagefault::complete_fault(7,9,token).unwrap();
 assert_eq!(RESUMES.load(Ordering::Relaxed),1);assert!(!unsafe{(*task).state.snapshot().fault_blocked});assert_eq!(unsafe{(*task).context.rip},0x100234);
 assert_eq!(unsafe{(*task).state.park_fault(0)},ownership::ParkResult::Blocked);let next=forward(0x2000,4,7).unwrap();kernel_pagefault::complete_fault(7,9,token).unwrap();assert!(unsafe{(*task).state.snapshot().fault_blocked});assert_eq!(kernel_pagefault::query_fault(7,9,next).unwrap().token,next);
}
#[test] fn rejection_is_authenticated_and_terminal(){
 let _lock=SERIAL.lock().unwrap();let task=setup();let token=forward(0x1000,4,7).unwrap();assert_eq!(kernel_pagefault::reject_fault(7,8,token),Err(DENIED));kernel_pagefault::reject_fault(7,9,token).unwrap();assert!(unsafe{(*task).state.snapshot().exited});assert!(kernel_pagefault::complete_fault(7,9,token).is_err());assert_eq!(RESUMES.load(Ordering::Relaxed),0);
}
#[test]fn dead_fault_retirement_requires_released_exact_task_and_cannot_resume_old_frame(){
 let _lock=SERIAL.lock().unwrap();let task=setup();let token=forward(0x1000,6,7).unwrap();kernel_pagefault::claim_fault(7,9,token).unwrap();
 assert_eq!(kernel_pagefault::retire_dead_fault(7,9,0x80000007),Err(5));unsafe{(*task).state.exit();}
 assert_eq!(kernel_pagefault::retire_dead_fault(7,9,0x80000007),Err(RETRY));unsafe{(*task).state.begin_switch_out(0);(*task).state.release_after_stack_switch(0);}
 assert_eq!(kernel_pagefault::retire_dead_fault(7,9,7),Err(DENIED));assert_eq!(kernel_pagefault::retire_dead_fault(7,8,0x80000007),Err(DENIED));
 kernel_pagefault::retire_dead_fault(7,9,0x80000007).unwrap();kernel_pagefault::retire_dead_fault(7,9,0x80000007).unwrap();
 assert_eq!(kernel_pagefault::complete_fault(7,9,token),Err(INVALID));assert_eq!(RESUMES.load(Ordering::Relaxed),0);
}
