// SPDX-License-Identifier: GPL-3.0-only
//! Actual Browser transport and Close ABI; finite host syscall scripts only.
//! Unequal task IDs and future PD30 are observations, not installed authority.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../apps/kaleidoscope/src/service_call.rs"]mod service_call;
use std::{cell::RefCell,collections::VecDeque};
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[derive(Clone,Copy)]struct Identity{target:u32,task:u32}
impl Identity{fn target(self)->u32{self.target}fn task(self)->u32{self.task}}
#[derive(Clone,Copy,Debug)]pub struct Message{type_id:u64,caller_pd:u32,arg0:u64,arg1:u64,arg2:u64}
struct Script{identity:Identity,status:VecDeque<u64>,incoming:VecDeque<Message>,calls:Vec<(u64,u64,[u64;3])>,yields:usize,listens:usize,admitted:bool}
thread_local!{static SCRIPT:RefCell<Script>=RefCell::new(Script{identity:Identity{target:0,task:0},status:VecDeque::new(),incoming:VecDeque::new(),calls:vec![],yields:0,listens:0,admitted:false});}
fn browser_identity()->Identity{SCRIPT.with(|s|s.borrow().identity)}
pub fn pdx_call(slot:u64,opcode:u64,a:u64,b:u64,c:u64)->(u64,u64){SCRIPT.with(|s|{let mut s=s.borrow_mut();s.calls.push((slot,opcode,[a,b,c]));let status=s.status.pop_front().expect("unexpected enqueue/replay");s.admitted=status==0;(status,0xdead)})}
pub fn pdx_try_listen_raw(slot:u64)->Option<Message>{assert_eq!(slot,0);SCRIPT.with(|s|s.borrow_mut().incoming.pop_front())}
pub fn pdx_listen_raw(slot:u64)->Message{assert_eq!(slot,0);SCRIPT.with(|s|{let mut s=s.borrow_mut();assert!(s.admitted,"reply wait before admission");s.listens+=1;s.incoming.pop_front().expect("unexpected reply wait")})}
pub fn sys_yield(){SCRIPT.with(|s|s.borrow_mut().yields+=1)}
#[derive(Debug)]struct Finished(u64);
pub fn task_finish(code:u64)->!{std::panic::panic_any(Finished(code))}
fn reset(target:u32,task:u32,status:Vec<u64>,incoming:Vec<Message>){SCRIPT.with(|s|*s.borrow_mut()=Script{identity:Identity{target,task},status:status.into(),incoming:incoming.into(),calls:vec![],yields:0,listens:0,admitted:false});}
fn close(target:u32,task:u32)->Message{Message{type_id:app_lifecycle::OP_APP_CLOSE,caller_pd:3,arg0:target as u64,arg1:task as u64,arg2:0}}
fn reply(caller:u32,value:u64)->Message{Message{type_id:1,caller_pd:caller,arg0:value,arg1:0,arg2:0}}
fn terminated(f:impl FnOnce()){let e=std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("Close did not finish");assert_eq!(e.downcast_ref::<Finished>().expect("unexpected panic").0,0);}
#[test]fn exact_held_close_finishes_both_wait_paths_for_unequal_tasks(){
 for (target,task) in [(15,150),(30,300)]{for confirmed in [0,1]{
  let mut m=close(target,task);m.arg2=confirmed;
  reset(target,task,vec![ERR_SERVICE_NOT_READY],vec![m]);terminated(||{let _=service_call::enqueue(7,42,[1,2,3]);});
  SCRIPT.with(|s|{let s=s.borrow();assert_eq!(s.calls.len(),1);assert_eq!(s.yields,0);assert_eq!(s.listens,0);});
  reset(target,task,vec![0],vec![m]);terminated(||{let _=service_call::request(7,11,42,[1,2,3]);});
  SCRIPT.with(|s|assert_eq!(s.borrow().listens,1));
 }}
}
fn false_closes(target:u32,task:u32)->Vec<Message>{
 let exact=close(target,task);let mut out=vec![close(target,target),close(15,15),close(target+1,task),close(target,task+1)];
 for kind in 0..5{let mut m=exact;match kind{0=>m.type_id=1,1=>m.caller_pd=11,2=>m.arg2=2,3=>m.arg0|=1<<32,_=>m.arg1|=1<<32}out.push(m);}out
}
#[test]fn stale_wrong_type_target_task_and_coordinator_never_close_or_reply(){
 for (target,task) in [(15,150),(30,300)]{for m in false_closes(target,task){
  reset(target,task,vec![ERR_SERVICE_NOT_READY,0],vec![m]);assert_eq!(service_call::enqueue(7,42,[1,2,3]),Ok(()));
  SCRIPT.with(|s|{let s=s.borrow();assert_eq!(s.yields,1);assert_eq!(s.calls,vec![(7,42,[1,2,3]);2]);});
  reset(target,task,vec![0],vec![m,reply(12,8),reply(11,9)]);assert_eq!(service_call::request(7,11,42,[1,2,3]),Ok(9));
  SCRIPT.with(|s|assert_eq!(s.borrow().listens,3));
 }}
}
#[test]fn full_retries_exact_request_and_discards_pre_admission_provider_reply(){
 reset(15,150,vec![ERR_SERVICE_NOT_READY,ERR_SERVICE_NOT_READY,0],vec![reply(11,111),close(15,15),reply(11,222)]);
 assert_eq!(service_call::request(7,11,42,[5,6,7]),Ok(222));
 SCRIPT.with(|s|{let s=s.borrow();assert_eq!(s.calls,vec![(7,42,[5,6,7]);3]);assert_eq!(s.yields,2);assert_eq!(s.listens,1);});
}
#[test]fn permanent_enqueue_denial_returns_without_receiving_or_replaying(){
 for denied in [1,3,u64::MAX]{
  reset(30,300,vec![denied],vec![close(30,300)]);assert_eq!(service_call::request(7,11,42,[1,2,3]),Err(denied));
  SCRIPT.with(|s|{let s=s.borrow();assert_eq!(s.calls.len(),1);assert_eq!(s.incoming.len(),1);assert_eq!(s.listens,0);assert_eq!(s.yields,0);});
 }
}
#[test]fn serialized_calls_each_admit_once_before_consuming_own_provider_reply(){
 reset(30,300,vec![0,ERR_SERVICE_NOT_READY,0],vec![reply(11,1),reply(11,999),reply(11,2)]);
 assert_eq!(service_call::request(7,11,42,[1,0,0]),Ok(1));
 assert_eq!(service_call::request(7,11,42,[2,0,0]),Ok(2));
 SCRIPT.with(|s|{let s=s.borrow();assert_eq!(s.calls,[(7,42,[1,0,0]),(7,42,[2,0,0]),(7,42,[2,0,0])]);assert!(s.incoming.is_empty());});
}
