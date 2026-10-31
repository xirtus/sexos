// SPDX-License-Identifier: GPL-3.0-only
//! Actual scheduler-context sender capture, catalog comparison and public ABI
//! layout; scheduler/PD boundary doubles do not establish native containment.
#![allow(dead_code)]
use std::sync::{Mutex,atomic::{AtomicPtr,AtomicU32,Ordering}};
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../kernel/src/ipc/sender.rs"]mod sender;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod app_lifecycle;
include!(env!("IPC_MESSAGE_SOURCE"));
static LOCK:Mutex<()>=Mutex::new(());
static DOMAIN:AtomicPtr<Domain>=AtomicPtr::new(core::ptr::null_mut());
static CORE:AtomicPtr<core_local::CoreLocal>=AtomicPtr::new(core::ptr::null_mut());
static PD_ID:AtomicU32=AtomicU32::new(0);
struct Domain{id:u32}
mod core_local{
 pub struct CoreLocal{pub core_id:u32}
 impl CoreLocal{
  pub fn get()->&'static Self{unsafe{super::CORE.load(super::Ordering::Acquire).as_ref().unwrap()}}
  pub fn current_pd(&self)->u32{super::PD_ID.load(super::Ordering::Acquire)}
  pub fn current_pd_ref(&self)->&'static super::Domain{unsafe{super::DOMAIN.load(super::Ordering::Acquire).as_ref().unwrap()}}
 }
}
mod scheduler{
 use super::*;
 pub struct Context{pub pd_id:u64,pub pd_ptr:*const Domain}
 pub struct Snapshot{pub exited:bool}
 pub struct State{pub exited:bool}
 impl State{pub fn snapshot(&self)->Snapshot{Snapshot{exited:self.exited}}}
 pub struct Task{pub id:u32,pub context:Context,pub state:State}
 pub struct Scheduler{pub current_task:AtomicPtr<Task>}
 pub static SCHEDULERS:[Scheduler;1]=[Scheduler{current_task:AtomicPtr::new(core::ptr::null_mut())}];
}
fn setup(pd:u32,task:u32)->(*mut Domain,*mut scheduler::Task){
 let domain=Box::into_raw(Box::new(Domain{id:pd}));DOMAIN.store(domain,Ordering::Release);PD_ID.store(pd,Ordering::Release);
 CORE.store(Box::into_raw(Box::new(core_local::CoreLocal{core_id:0})),Ordering::Release);
 let task=Box::into_raw(Box::new(scheduler::Task{id:task,context:scheduler::Context{pd_id:pd as u64,pd_ptr:domain},state:scheduler::State{exited:false}}));
 scheduler::SCHEDULERS[0].current_task.store(task,Ordering::Release);(domain,task)
}
#[test]fn captures_actual_scheduled_task_not_domain_or_catalog_number(){let _l=LOCK.lock().unwrap();setup(15,901);assert_eq!(sender::current_task(15),Some(901));}
#[test]fn missing_scheduled_task_cannot_attest_identity(){let _l=LOCK.lock().unwrap();setup(15,901);scheduler::SCHEDULERS[0].current_task.store(core::ptr::null_mut(),Ordering::Release);assert_eq!(sender::current_task(15),None);}
#[test]fn zero_task_and_out_of_range_core_fail_closed(){let _l=LOCK.lock().unwrap();setup(15,0);assert_eq!(sender::current_task(15),None);setup(15,901);CORE.store(Box::into_raw(Box::new(core_local::CoreLocal{core_id:1})),Ordering::Release);assert_eq!(sender::current_task(15),None);}
#[test]fn current_domain_scalar_and_pointer_must_match_expected_sender(){let _l=LOCK.lock().unwrap();setup(15,901);assert_eq!(sender::current_task(16),None);PD_ID.store(16,Ordering::Release);assert_eq!(sender::current_task(15),None);}
#[test]fn task_context_full_width_domain_cannot_alias_or_substitute(){let _l=LOCK.lock().unwrap();let(_,t)=setup(15,901);for pd in[16,15+(1u64<<32),u64::MAX]{unsafe{(*t).context.pd_id=pd};assert_eq!(sender::current_task(15),None);}}
#[test]fn equal_numeric_domain_does_not_substitute_another_domain_pointer(){let _l=LOCK.lock().unwrap();let(_,t)=setup(15,901);unsafe{(*t).context.pd_ptr=Box::into_raw(Box::new(Domain{id:15}))};assert_eq!(sender::current_task(15),None);}
#[test]fn terminal_task_cannot_supply_new_application_provenance(){let _l=LOCK.lock().unwrap();let(_,t)=setup(15,901);unsafe{(*t).state.exited=true};assert_eq!(sender::current_task(15),None);}
#[test]fn captured_scalar_is_stable_after_scheduler_moves_to_another_task(){let _l=LOCK.lock().unwrap();setup(15,901);let old=sender::current_task(15).unwrap();setup(15,902);assert_eq!((old,sender::current_task(15)),(901,Some(902)));}
#[test]fn catalog_checks_actual_task_separately_from_expected_domain(){
 let c=app_lifecycle::CatalogInfo{target_pd:15,task_id:901,surface_id:300,coordinator_pd:3};assert!(c.is_sender(15,901));
 for(pd,task)in[(15,15),(15,902),(16,901),(0,901),(15,0),(0,0)]{assert!(!c.is_sender(pd,task));}
 assert!(!app_lifecycle::CatalogInfo{task_id:0,..c}.is_sender(15,0));
}
#[test]fn task_metadata_fills_old_padding_without_changing_message_layout(){
 assert_eq!(core::mem::size_of::<PdxMessage>(),40);assert_eq!(core::mem::align_of::<PdxMessage>(),8);
 assert_eq!(core::mem::offset_of!(PdxMessage,caller_pd),32);assert_eq!(core::mem::offset_of!(PdxMessage,caller_task),36);
 let m=PdxMessage{type_id:77,arg0:u64::MAX,arg1:901,arg2:15,caller_pd:15,caller_task:902};assert_eq!((m.caller_pd,m.caller_task),(15,902));
}
