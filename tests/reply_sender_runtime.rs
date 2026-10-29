// SPDX-License-Identifier: GPL-3.0-only
//! Production reply router and message definitions with registry/core/scheduler
//! boundary doubles. This proves provenance/retention, not return-cap authority.
#![allow(dead_code)]
#[path="../kernel/src/ipc/messages.rs"] pub mod messages;
#[path="../kernel/src/ipc/router.rs"] mod router;
use std::{cell::Cell,collections::VecDeque,sync::{Mutex,MutexGuard,atomic::{AtomicPtr,AtomicUsize,Ordering}}};
static SERIAL:Mutex<()>=Mutex::new(());
static TARGET:AtomicPtr<Domain>=AtomicPtr::new(core::ptr::null_mut());
static WAKES:AtomicUsize=AtomicUsize::new(0);
struct ReplyLock(Mutex<VecDeque<messages::IpcReply>>);
struct ReplyGuard<'a>(MutexGuard<'a,VecDeque<messages::IpcReply>>);
impl core::ops::Deref for ReplyGuard<'_>{type Target=VecDeque<messages::IpcReply>;fn deref(&self)->&Self::Target{&self.0}}
impl core::ops::DerefMut for ReplyGuard<'_>{fn deref_mut(&mut self)->&mut Self::Target{&mut self.0}}
impl Drop for ReplyGuard<'_>{fn drop(&mut self){REPLY_HELD.with(|held|held.set(false));}}
impl ReplyLock{fn lock(&self)->ReplyGuard<'_>{let guard=self.0.lock().unwrap();REPLY_HELD.with(|held|{assert!(!held.replace(true));});ReplyGuard(guard)}}
struct Ring;
impl Ring{fn enqueue(&self,_message:messages::MessageType)->Result<(),()>{Ok(())}}
struct Domain{id:u32,incoming_replies:ReplyLock,main_task:AtomicPtr<scheduler::Task>,trampoline_task:AtomicPtr<scheduler::Task>,message_ring:*mut Ring}
unsafe impl Sync for Domain{}
unsafe impl Send for Domain{}
fn domain(id:u32)->&'static Domain{Box::leak(Box::new(Domain{id,incoming_replies:ReplyLock(Mutex::new(VecDeque::new())),main_task:AtomicPtr::new(core::ptr::null_mut()),trampoline_task:AtomicPtr::new(core::ptr::null_mut()),message_ring:Box::into_raw(Box::new(Ring))}))}
thread_local!{static REPLY_HELD:Cell<bool>=const{Cell::new(false)};static CALLER:Cell<*const Domain>=const{Cell::new(core::ptr::null())};}
mod app_resources{
 pub static ACCEPT:core::sync::atomic::AtomicBool=core::sync::atomic::AtomicBool::new(true);
 pub fn accepts_ipc(_:u32)->bool{ACCEPT.load(super::Ordering::Acquire)}
}
mod core_local{
 pub struct CoreLocal;static CORE:CoreLocal=CoreLocal;
 impl CoreLocal{pub fn get()->&'static Self{&CORE}pub fn current_pd_ref(&self)->&'static super::Domain{super::CALLER.with(|p|unsafe{p.get().as_ref().expect("fixture caller")})}}
}
mod ipc{
 pub use super::messages;
 pub struct Registry;pub static DOMAIN_REGISTRY:Registry=Registry;
 impl Registry{pub fn get(&self,id:u32)->Option<&'static super::Domain>{let p=super::TARGET.load(super::Ordering::Acquire);unsafe{p.as_ref().filter(|p|p.id==id)}}}
}
mod capabilities{pub mod engine{pub struct CapEngine;impl CapEngine{pub fn verify_signal_rights(_pd:&super::super::Domain,_cap:u64)->bool{false}}}}
mod scheduler{
 pub struct Task;
 pub fn unpark_thread(task:*mut Task){
  let pd=unsafe{&*super::TARGET.load(super::Ordering::Acquire)};
  assert_eq!(task,pd.main_task.load(super::Ordering::Acquire));
  // This sender must release its own lock before waking. A different racing
  // sender may legitimately hold the same lock by then; try_lock confused
  // that contention with a publication bug and made this test flaky.
  super::REPLY_HELD.with(|held|assert!(!held.get(),"wake while this sender holds reply lock"));
  let replies=pd.incoming_replies.lock();
  assert_eq!(replies.len(),1);assert!(replies[0].caller_pd>0);
  super::WAKES.fetch_add(1,super::Ordering::Relaxed);
 }
}
fn setup(sender:u32)->&'static Domain{
 app_resources::ACCEPT.store(true,Ordering::Release);
 let pd=domain(15);pd.main_task.store(Box::into_raw(Box::new(scheduler::Task)),Ordering::Release);
 TARGET.store(pd as *const _ as *mut _,Ordering::Release);WAKES.store(0,Ordering::Relaxed);CALLER.with(|p|p.set(domain(sender)));pd
}
#[test]fn sender_is_captured_from_current_pd_not_target_or_value(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);
 assert_eq!(router::send_reply(15,0xdeadbeef),Ok(()));CALLER.with(|p|p.set(domain(10)));
 let reply=pd.incoming_replies.lock().pop_front().unwrap();assert_eq!((reply.caller_pd,reply.value),(14,0xdeadbeef));
 assert_eq!(router::send_reply(15,14),Ok(()));let reply=pd.incoming_replies.lock().pop_front().unwrap();assert_eq!((reply.caller_pd,reply.value),(10,14));assert_eq!(WAKES.load(Ordering::Relaxed),2);
}
#[test]fn full_slot_retains_old_reply_and_retry_publishes_only_after_consumption(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);router::send_reply(15,0x100).unwrap();CALLER.with(|p|p.set(domain(10)));
 for _ in 0..20{assert_eq!(router::send_reply(15,0x200),Err(messages::IpcError::BufferFull));}
 assert_eq!(WAKES.load(Ordering::Relaxed),1);let old=pd.incoming_replies.lock().pop_front().unwrap();assert_eq!((old.caller_pd,old.value),(14,0x100));
 router::send_reply(15,0x200).unwrap();let new=pd.incoming_replies.lock().pop_front().unwrap();assert_eq!((new.caller_pd,new.value),(10,0x200));assert_eq!(WAKES.load(Ordering::Relaxed),2);
}
#[test]fn invalid_targets_neither_enqueue_nor_wake(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);
 for target in [0,1,14,16,u32::MAX]{assert_eq!(router::send_reply(target,7),Err(messages::IpcError::InvalidTarget));}
 assert!(pd.incoming_replies.lock().is_empty());assert_eq!(WAKES.load(Ordering::Relaxed),0);
}
#[test]fn missing_main_task_keeps_reply_without_inventing_wake(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);pd.main_task.store(core::ptr::null_mut(),Ordering::Release);
 router::send_reply(15,42).unwrap();assert_eq!(pd.incoming_replies.lock()[0].caller_pd,14);assert_eq!(WAKES.load(Ordering::Relaxed),0);
}
#[test]fn concurrent_senders_have_one_winner_and_no_overwrite(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);let barrier=std::sync::Arc::new(std::sync::Barrier::new(16));
 let workers:Vec<_>=(1..=16u32).map(|sender|{let barrier=barrier.clone();std::thread::spawn(move||{CALLER.with(|p|p.set(domain(sender)));barrier.wait();(sender,router::send_reply(15,sender as u64+1000))})}).collect();
 let results:Vec<_>=workers.into_iter().map(|w|w.join().unwrap()).collect();let wins:Vec<_>=results.iter().filter(|(_,result)|result.is_ok()).collect();assert_eq!(wins.len(),1);
 assert!(results.iter().filter(|(_,r)|r.is_err()).all(|(_,r)|*r==Err(messages::IpcError::BufferFull)));
 let reply=pd.incoming_replies.lock().pop_front().unwrap();assert_eq!(reply.caller_pd,wins[0].0);assert_eq!(reply.value,wins[0].0 as u64+1000);assert_eq!(WAKES.load(Ordering::Relaxed),1);
}

#[test]fn retired_target_keeps_permanent_metadata_but_refuses_new_delivery(){
 let _lock=SERIAL.lock().unwrap();let pd=setup(14);app_resources::ACCEPT.store(false,Ordering::Release);
 assert_eq!(router::send_reply(15,7),Err(messages::IpcError::InvalidTarget));
 assert!(router::route_signal(14,15,1,3).is_err());
 assert!(pd.incoming_replies.lock().is_empty());assert_eq!(WAKES.load(Ordering::Relaxed),0);
}
