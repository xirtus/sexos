// SPDX-License-Identifier: GPL-3.0-only
//! Actual bounded exit watcher + TaskOwnership. Fake registry and unpark only;
//! physical stack departure remains an explicit unsafe test boundary.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/paging.rs"] pub mod paging;
#[path="../kernel/src/task_ownership.rs"] pub mod ownership;
#[path="../kernel/src/ipc/messages.rs"] pub mod messages;
#[path="../kernel/src/ipc_ring.rs"] mod ring;
#[path="../kernel/src/task_exit.rs"] mod exit;
use std::sync::{Mutex,atomic::{AtomicPtr,AtomicU8,AtomicUsize,Ordering}};
use ownership::{TaskOwnership,Admission,ParkResult,Phase};
#[macro_export] macro_rules! serial_println{($($arg:tt)*)=>{let _=format_args!($($arg)*);};}
static SERIAL:Mutex<()>=Mutex::new(());
static DOMAINS:[AtomicPtr<Domain>;16]=[const{AtomicPtr::new(core::ptr::null_mut())};16];
static WAKES:AtomicUsize=AtomicUsize::new(0);
struct Domain{main_task:AtomicPtr<scheduler::Task>,trampoline_task:AtomicPtr<scheduler::Task>,message_ring:*mut ring::RingBuffer<messages::MessageType,4>}
mod app_resources{pub fn poll_exit_notifications(){}}
mod scheduler{
 pub use super::ownership as task_ownership;
 pub struct Task{pub id:u32,pub state:super::TaskOwnership}
 pub fn unpark_thread(task:*mut Task){if !task.is_null(){unsafe{(*task).state.wake();}super::WAKES.fetch_add(1,super::Ordering::Relaxed);}}
}
mod ipc{
 pub use super::messages;
 #[repr(u8)]pub enum BootPhase{RegistryBuild=1}
 pub static BOOT_PHASE:super::AtomicU8=super::AtomicU8::new(1);
 pub struct Registry;pub static DOMAIN_REGISTRY:Registry=Registry;
 impl Registry{pub fn get(&self,id:u32)->Option<&'static super::Domain>{super::DOMAINS.get(id as usize).and_then(|p|unsafe{p.load(super::Ordering::Acquire).as_ref()})}}
}
fn attached()->TaskOwnership{let t=TaskOwnership::new();assert!(t.activate());assert_eq!(t.try_admit(||Ok::<(),()>(())),Admission::Published);assert!(t.begin_dispatch(0));assert!(t.attach(0));t}
fn domain(id:u32,state:TaskOwnership)->(&'static Domain,*mut scheduler::Task){
 let task=Box::into_raw(Box::new(scheduler::Task{id:0x8000+id,state}));
 let d=Box::leak(Box::new(Domain{main_task:AtomicPtr::new(task),trampoline_task:AtomicPtr::new(core::ptr::null_mut()),message_ring:Box::into_raw(Box::new(ring::RingBuffer::new()))}));
 DOMAINS[id as usize].store(d,Ordering::Release);(d,task)
}
#[test]fn last_runnable_exit_kicks_pager_but_notifies_only_after_stack_release_and_retains_full_queue(){
 let _lock=SERIAL.lock().unwrap();WAKES.store(0,Ordering::Relaxed);
 let pager=attached();assert_eq!(pager.park(0),ParkResult::Blocked);assert!(pager.begin_switch_out(0));assert!(unsafe{pager.release_after_stack_switch(0)});
 let(pd,pt)=domain(14,pager);let(_,task)=domain(15,attached());exit::register_paging_exit_watch(15,14);
 assert_eq!(exit::validate_dead_target(15,14),Err(5));assert_eq!(exit::validate_dead_target(15,13),Err(3));
 unsafe{(*task).state.exit();}assert_eq!(exit::validate_dead_target(15,14),Err(2));exit::poll_paging_exit_notifications();
 assert_eq!(WAKES.load(Ordering::Relaxed),1);assert!(unsafe{(*pt).state.snapshot().pending});assert!(unsafe{(*pd.message_ring).dequeue()}.is_none());
 unsafe{assert!((*task).state.begin_switch_out(0));}exit::poll_paging_exit_notifications();assert!(unsafe{(*pd.message_ring).dequeue()}.is_none());assert_eq!(exit::validate_dead_target(15,14),Err(2));
 unsafe{assert!((*task).state.release_after_stack_switch(0));}assert_eq!(exit::validate_dead_target(15,14).unwrap().task_id,0x800f);
 for _ in 0..4{unsafe{(*pd.message_ring).enqueue(messages::MessageType::Empty).unwrap();}}
 for _ in 0..8{exit::poll_paging_exit_notifications();}assert_eq!(WAKES.load(Ordering::Relaxed),1);
 unsafe{(*pd.message_ring).dequeue();}exit::poll_paging_exit_notifications();assert_eq!(WAKES.load(Ordering::Relaxed),2);
 let mut notices=0;while let Some(m)=unsafe{(*pd.message_ring).dequeue()}{if let messages::MessageType::IpcCall{func_id,arg0,arg1,arg2,caller_pd,caller_task}=m{assert_eq!((func_id,arg0,arg1,arg2,caller_pd,caller_task),(paging::OP_TASK_EXIT,15,0x800f,0,0,0));notices+=1;}}assert_eq!(notices,1);
 for _ in 0..8{exit::poll_paging_exit_notifications();}assert!(unsafe{(*pd.message_ring).dequeue()}.is_none());
 assert!(!exit::acknowledge_dead_cleanup(15,14,15));assert!(exit::acknowledge_dead_cleanup(15,14,0x800f));exit::poll_paging_exit_notifications();assert_eq!(WAKES.load(Ordering::Relaxed),2);
}
#[test]fn queued_exit_waits_for_token_and_active_trampoline_blocks_cleanup(){
 let _lock=SERIAL.lock().unwrap();if ipc::DOMAIN_REGISTRY.get(14).is_none(){domain(14,attached());}
 let state=TaskOwnership::new();state.activate();assert_eq!(state.try_admit(||Ok::<(),()>(())),Admission::Published);
 let(pd,task)=domain(10,state);exit::register_paging_exit_watch(10,14);unsafe{(*task).state.exit();}
 assert_eq!(exit::validate_dead_target(10,14),Err(2));assert!(!unsafe{(*task).state.begin_dispatch(0)});
 assert!(exit::validate_dead_target(10,14).is_ok());let trampoline=Box::into_raw(Box::new(scheduler::Task{id:0xdead,state:attached()}));pd.trampoline_task.store(trampoline,Ordering::Release);
 assert_eq!(exit::validate_dead_target(10,14),Err(2));unsafe{(*trampoline).state.exit();(*trampoline).state.begin_switch_out(0);(*trampoline).state.release_after_stack_switch(0);}
 assert!(exit::validate_dead_target(10,14).is_ok());let replacement=Box::into_raw(Box::new(scheduler::Task{id:0x800a,state:TaskOwnership::new()}));pd.main_task.store(replacement,Ordering::Release);assert_eq!(exit::validate_dead_target(10,14),Err(3));pd.main_task.store(task,Ordering::Release);
 assert!(exit::acknowledge_dead_cleanup(10,14,0x800a));
}

#[test]fn ordinary_exit_never_allows_local_resume_or_stale_wake(){
 let state=attached();assert!(state.exit());assert!(!state.resume_attached(0));
 assert_eq!(state.wake(),ownership::WakeResult::Exited);assert_eq!(state.snapshot().phase,Phase::Attached);
 assert!(state.begin_switch_out(0));assert!(!unsafe{state.release_after_stack_switch(1)});
 assert_eq!(state.snapshot().phase,Phase::SwitchingOut);assert!(unsafe{state.release_after_stack_switch(0)});
 assert_eq!(state.snapshot().phase,Phase::Dead);assert_eq!(state.snapshot().owner_core,None);
 assert_eq!(state.try_admit::<()>(||panic!("dead task must not publish")),Admission::NotRunnable);
}
#[test]fn constructed_detached_and_dispatching_exit_preserve_exact_custody(){
 let constructed=TaskOwnership::new();assert!(constructed.exit());assert_eq!(constructed.snapshot().phase,Phase::Dead);
 let detached=TaskOwnership::new();detached.activate();assert!(detached.exit());assert_eq!(detached.snapshot().phase,Phase::Dead);
 let dispatch=TaskOwnership::new();dispatch.activate();dispatch.try_admit(||Ok::<(),()>(()));assert!(dispatch.begin_dispatch(0));dispatch.exit();
 assert_eq!(dispatch.snapshot().phase,Phase::Dispatching);assert!(!dispatch.attach(0));assert_eq!(dispatch.snapshot().phase,Phase::Dead);
}
