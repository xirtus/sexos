"""Actual Shell held-route predicates and delivery hooks, with typed syscall stubs."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
class NativeShellRole(unittest.TestCase):
 def test_actual_native_guards_and_wiring(self):
  main=(ROOT/'servers/silk-shell/src/main.rs').read_text()
  intake=main[main.index('let mut msg = match pop_startup_message()'):]
  self.assertLess(intake.index('restricted_routes::receive(&msg)'),intake.index('shell_launch_message(&msg)'))
  self.assertLess(intake.index('restricted_pool::blocks(&msg)'),intake.index('autofill_routes::receive(&mut msg)'))
  cleanup=main[main.index('unsafe fn shell_lifecycle_poll()'):main.index('unsafe fn shell_lifecycle_poll()')+5000]
  self.assertLess(cleanup.index('restricted_routes::retire'),cleanup.index('app_lifecycle::ack'))
  self.assertIn('!restricted_routes::busy()&&!restricted_pool::reserved(row.target)',main)
  with tempfile.TemporaryDirectory() as d:
   folder=Path(d);text='extern crate self as sex_pdx;\n'
   for name,path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'),('restricted_helper_journal','crates/sex-pdx/src/restricted_helper_journal.rs'),('restricted_job','crates/sex-pdx/src/restricted_job.rs'),('restricted_coordinator','servers/silk-shell/src/restricted_coordinator.rs'),('app_catalog','crates/sex-pdx/src/app_catalog.rs'),('program_store','crates/sex-pdx/src/program_store.rs'),('launch','crates/sex-pdx/src/launch.rs')]:
    text+='#[path="'+str(ROOT/path)+'"]mod '+name+';\n'
   pool=(ROOT/'servers/silk-shell/src/restricted_pool.rs').read_text().replace('#[cfg(target_os = "none")]','').replace('//!','//')
   text+='mod restricted_pool{'+pool+'}\n'
   text+=r'''
use core::sync::atomic::{AtomicBool,AtomicU32,Ordering};use restricted_helper::Subject;use restricted_coordinator::Platform;
const SLOT_DISPLAY:u64=5;const SLOT_BELL:u64=12;mod web{pub const SHELL_SLOT:u64=262;}mod package{pub const GRANT_TOKEN:&[u8]=b"+pkg";}
#[derive(Clone,Copy)]struct PdxMessage{type_id:u64,caller_pd:u32,caller_task:u32,arg0:u64,arg1:u64,arg2:u64}
#[derive(Clone,Copy)]struct Entry{target:u32,task:u32,dead:bool,close_pending:bool}
static mut APP_CUSTODY:[Option<Entry>;2]=[Some(Entry{target:42,task:99,dead:false,close_pending:false}),Some(Entry{target:43,task:43,dead:false,close_pending:false})];
static mut PROGRAM_READING:bool=false;static mut PROGRAM_REBIRTH:Option<usize>=None;static mut RECORD_SPAWN_LEFT:u64=0;
#[derive(Clone,Copy)]struct LaunchRun{parent:u32,parent_task:u32,cookie:u32,child:u32,child_task:u32,status:Option<u32>,ended:Option<u64>}
static mut LAUNCH_RUNS:[Option<LaunchRun>;16]=[None;16];static mut HIDDEN_RUNS:[Option<(u32,u32)>;16]=[None;16];
static TASK:AtomicU32=AtomicU32::new(43);static PARENT:AtomicU32=AtomicU32::new(99);static STATE:AtomicU32=AtomicU32::new(1);static FORCED:AtomicU32=AtomicU32::new(0);static SENT:AtomicU32=AtomicU32::new(0);static REPLIED:AtomicU32=AtomicU32::new(0);static FULL:AtomicBool=AtomicBool::new(false);
fn get_ticks()->u64{100}fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){if FULL.load(Ordering::SeqCst){return(1,0)}SENT.fetch_add(1,Ordering::SeqCst);(0,0)}fn pdx_reply(_:u32,_:u64)->u64{REPLIED.fetch_add(1,Ordering::SeqCst);0}
mod crash_log{pub fn self_epoch()->Result<u64,u64>{Ok(1)}pub fn keep(_:u64,_:u64)->bool{true}pub struct Peer{pub epoch:u64}impl Peer{pub fn live(&self)->bool{true}}pub fn peer_state(_:u64)->Result<Peer,u64>{Ok(Peer{epoch:1})}}
mod app_lifecycle{
 use super::*;pub const ACTIVATION_READY:u64=1;pub const ACTIVATION_ACTIVE:u64=2;pub const ACTIVATION_REUSABLE:u64=4;pub const ACTIVATION_REBIRTHING:u64=5;pub const RETRY:u64=6;
 pub struct Catalog{pub task_id:u32,pub target_pd:u32,pub surface_id:u64}impl Catalog{pub fn is_subject(&self,p:u32,s:u64,c:u32)->bool{p==self.target_pd&&s==self.surface_id&&c==3}}
 pub struct Activation{pub successor:u32,pub surface:u64,pub task:u32,pub state:u64}
 fn task(pd:u32)->u32{if pd==43{TASK.load(Ordering::SeqCst)}else if pd==42{PARENT.load(Ordering::SeqCst)}else{pd}}
 pub fn catalog(slot:usize)->Result<Catalog,u64>{let r=app_catalog::INCARNATIONS.iter().find(|r|r.lifecycle_slot==slot).ok_or(9u64)?;Ok(Catalog{task_id:task(r.target),target_pd:r.target,surface_id:r.surface})}
 pub fn activation(slot:usize,commit:bool)->Result<Activation,u64>{let r=app_catalog::INCARNATIONS.iter().find(|r|r.activation_slot==Some(slot)).ok_or(9u64)?;let mut state=if r.target==43{STATE.load(Ordering::SeqCst)as u64}else if r.target==42{2}else{0};if commit&&r.target==43&&state==1{STATE.store(2,Ordering::SeqCst);state=2;}Ok(Activation{successor:r.target,surface:r.surface,task:task(r.target),state})}
 pub fn load_program(slot:usize,_:u32)->Result<Activation,u64>{activation(slot,false)}pub fn rebirth(slot:usize)->Result<Activation,u64>{activation(slot,true)}pub fn force_end(_:usize)->Result<(),u64>{FORCED.fetch_add(1,Ordering::SeqCst);Ok(())}
}
'''
   native=(ROOT/'servers/silk-shell/src/restricted_routes.rs').read_text().replace('//!','//')
   text+='mod restricted_routes{'+native+r'''
#[cfg(test)]mod tests{
use super::*;use core::sync::atomic::Ordering;
#[test]fn held_subject_guards_reject_reuse_and_role_escalation_before_delivery(){
 crate::restricted_pool::initialize();let mut kernel=Kernel;let r=kernel.reserve().unwrap();assert_eq!((r.previous.pd,r.previous.task,r.slot),(43,43,250));assert!(crate::restricted_pool::reserved(43));
 let child=kernel.copy(r,100).unwrap();assert_eq!(child,Subject{pd:43,task:43});assert!(kernel.bound(r,child));assert!(matches!(kernel.activate(r,child),Activation::Active));
 kernel.kill(Subject{task:44,..child});assert_eq!(crate::FORCED.load(Ordering::SeqCst),0);kernel.kill(child);assert_eq!(crate::FORCED.load(Ordering::SeqCst),1);
 assert!(!retired(child));crate::STATE.store(4,Ordering::SeqCst);assert!(retired(child));crate::TASK.store(44,Ordering::SeqCst);assert!(!retired(child));assert!(!kernel.ready(child,0));assert_eq!(crate::REPLIED.load(Ordering::SeqCst),0);
 crate::STATE.store(2,Ordering::SeqCst);let denied=crate::PdxMessage{type_id:job::OP_START,caller_pd:43,caller_task:44,arg0:7,arg1:9,arg2:1};assert!(receive(&denied));assert_eq!(crate::SENT.load(Ordering::SeqCst),0);
 let old=job::start_words(7,9,1,2).unwrap();assert!(receive(&crate::PdxMessage{type_id:job::OP_START,caller_pd:42,caller_task:99,arg0:old[0],arg1:old[1],arg2:old[2]}));assert!(!busy());assert_eq!(crate::SENT.load(Ordering::SeqCst),0);
 let stale=crate::PdxMessage{caller_pd:42,caller_task:98,..denied};assert!(receive(&stale));assert_eq!(crate::SENT.load(Ordering::SeqCst),0);
 let chunk=wire::chunk(7,5,1,0,0xa8ffffff).unwrap();assert!(receive(&crate::PdxMessage{type_id:1,caller_pd:11,caller_task:11,arg0:chunk,arg1:0,arg2:0}));assert!(!busy());

 // Unknown/recovery completion never claims retirement; exact lifecycle only.
 let query=job::status_words(9,None,1).unwrap();status(Subject{pd:42,task:99},query);let box_=unsafe{&mut *core::ptr::addr_of_mut!(STATUSBOX)};let index=box_.iter().position(Option::is_some).unwrap();let value=box_[index].unwrap().value;assert_eq!(value.state,job::STATUS_UNKNOWN);assert!(value.current.is_none());box_[index]=None;
 let known=Subject{pd:43,task:44};let query=job::status_words(9,Some(known),1).unwrap();status(Subject{pd:42,task:99},query);let value=box_[index].unwrap().value;assert_eq!(value.state,job::STATUS_LIVE);box_[index]=None;
 crate::STATE.store(1,Ordering::SeqCst);status(Subject{pd:42,task:99},query);assert_eq!(box_[index].unwrap().value.state,job::STATUS_UNAVAILABLE);box_[index]=None;
 crate::STATE.store(4,Ordering::SeqCst);status(Subject{pd:42,task:99},query);assert_eq!(box_[index].unwrap().value.state,job::STATUS_RETIRED);box_[index]=None;
 crate::TASK.store(45,Ordering::SeqCst);crate::STATE.store(2,Ordering::SeqCst);status(Subject{pd:42,task:99},query);assert_eq!(box_[index].unwrap().value.state,job::STATUS_RETIRED);box_[index]=None;
 for bad in [[query[0]|1<<32,query[1],1],[query[0],query[1],2],[query[0],Subject{pd:42,task:99}.word(),1],[0,query[1],1]] {status(Subject{pd:42,task:99},bad);assert!(box_.iter().all(Option::is_none));}
 status(Subject{pd:42,task:98},query);assert!(box_.iter().all(Option::is_none));crate::TASK.store(44,Ordering::SeqCst);
 // A retained refusal belongs to the old exact parent, never its reused row.
 crate::FULL.store(true,Ordering::SeqCst);refuse(Subject{pd:42,task:99},9);poll();assert!(pending());crate::PARENT.store(100,Ordering::SeqCst);crate::FULL.store(false,Ordering::SeqCst);poll();assert!(!pending());assert_eq!(crate::SENT.load(Ordering::SeqCst),0);
 crate::PARENT.store(99,Ordering::SeqCst);let start=job::start_words(7,9,1,1).unwrap();assert!(receive(&crate::PdxMessage{type_id:job::OP_START,caller_pd:42,caller_task:99,arg0:start[0],arg1:start[1],arg2:start[2]}));assert!(busy());poll();assert_eq!(crate::SENT.load(Ordering::SeqCst),1);
 for (pd,value)in [(11,0xf5u64<<56|1<<32),(1,0x5343_0000_0000_0001),(10,0),(10,0x4du64<<56)]{assert!(!receive(&crate::PdxMessage{type_id:1,caller_pd:pd,caller_task:pd,arg0:value,arg1:0,arg2:0}));assert!(busy());}
 retire(Subject{pd:42,task:99});assert!(!pending());
}
}
}
'''
   (folder/'test.rs').write_text(text)
   built=subprocess.run(['rustc','--edition=2021','--test',str(folder/'test.rs'),'-o',str(folder/'test')],capture_output=True,text=True)
   self.assertEqual(built.returncode,0,built.stdout+built.stderr)
   ran=subprocess.run([str(folder/'test')],capture_output=True,text=True)
   self.assertEqual(ran.returncode,0,ran.stdout+ran.stderr)
if __name__=='__main__':unittest.main()
