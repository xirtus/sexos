//! Full service restart module; explicit host registry, mapper and scheduler.
#![allow(dead_code)]
extern crate self as spin;
extern crate self as sex_pdx;
use std::{collections::VecDeque,sync::atomic::{AtomicPtr,Ordering}};
pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T>{pub const fn new(v:T)->Self{Self(std::sync::Mutex::new(v))}pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>>{self.0.try_lock().ok()}}
pub const SEXFS_V0_META_END_LBA:u64=47;pub const SEXDRIVE_AP4_MULTI_BASE_LBA:u64=128;
pub mod program_store{pub const DIRECTORY_LBA:u64=2048;}
#[path="../crates/sex-pdx/src/crash_log.rs"]mod wire;
pub mod crash_log{pub use crate::wire::*;pub fn poll(){}pub fn service(_:u32,_:u32,_:bool){}}
#[path="../kernel/src/capability_slot.rs"]mod capability_slot;
#[path="../kernel/src/task_ownership.rs"]pub mod task_ownership;
#[macro_export]macro_rules! serial_println{($($t:tt)*)=>{{let _=format_args!($($t)*);}};}
mod scheduler{
 pub use crate::task_ownership;pub struct Context{pub pd_id:u64,pub pd_ptr:*const crate::capability::ProtectionDomain,pub rip:u64,pub rsp:u64}
 pub struct Task{pub id:u32,pub context:Context,pub state:task_ownership::TaskOwnership}
 impl Task{pub fn rearm(&mut self,id:u32,entry:u64,stack:u64,pd:&crate::capability::ProtectionDomain){self.id=id;self.context=Context{pd_id:pd.id as u64,pd_ptr:pd,rip:entry,rsp:stack};self.state=task_ownership::TaskOwnership::new();}}
 pub struct Scheduler;pub static SCHEDULERS:[Scheduler;1]=[Scheduler];impl Scheduler{pub fn activate_task(&self,t:*mut Task)->bool{unsafe{(*t).state.activate()}}}pub fn unpark_thread(_: *mut Task){}
}
static REGISTRY:[AtomicPtr<capability::ProtectionDomain>;1024]=[const{AtomicPtr::new(std::ptr::null_mut())};1024];
mod capability{
 use crate::*;
 #[derive(Clone,Copy)]pub enum CapabilityData{Domain(u32),Memory}
 pub struct Capability{pub data:CapabilityData}
 pub struct ProtectionDomain{pub id:u32,pub pku_key:u8,pub main_task:AtomicPtr<scheduler::Task>,pub message_ring:*mut Queue,pub signal_ring:*mut Queue,pub incoming_replies:Mutex<VecDeque<u64>>,pub reply_credits:Mutex<Vec<u64>>,pub caps:Mutex<Vec<(u32,CapabilityData)>>}
 pub struct Queue;impl Queue{pub fn dequeue(&self)->Option<u8>{None}}
 impl ProtectionDomain{
  pub fn new(id:u32,task:u32)->&'static Self{let main=Box::into_raw(Box::new(scheduler::Task{id:task,context:scheduler::Context{pd_id:id as u64,pd_ptr:std::ptr::null(),rip:0x4000,rsp:0x9000},state:task_ownership::TaskOwnership::new()}));let pd=Box::leak(Box::new(Self{id,pku_key:1,main_task:AtomicPtr::new(main),message_ring:Box::into_raw(Box::new(Queue)),signal_ring:Box::into_raw(Box::new(Queue)),incoming_replies:Mutex::new(VecDeque::new()),reply_credits:Mutex::new(Vec::new()),caps:Mutex::new(Vec::new())}));unsafe{(*main).context.pd_ptr=pd;}REGISTRY[id as usize].store(pd,Ordering::Release);pd}
  pub fn find_capability(&self,slot:u32)->Option<Capability>{self.caps.lock().iter().find(|c|c.0==slot).map(|c|Capability{data:c.1})}
 }
}
mod ipc{
 pub const MAX_DOMAINS:usize=1024;
 pub struct Registry;pub static DOMAIN_REGISTRY:Registry=Registry;impl Registry{pub fn get(&self,id:u32)->Option<&'static crate::capability::ProtectionDomain>{let p=crate::REGISTRY.get(id as usize)?.load(crate::Ordering::Acquire);unsafe{p.as_ref()}}}
 pub mod messages{pub struct IpcReply;impl IpcReply{pub fn erase_queue(q:&mut std::collections::VecDeque<u64>){q.clear()}}}
}
mod boot_memory{pub fn existing_leaf_address(_:u64)->Option<u64>{None}pub fn backing(_:u64)->u64{0}pub fn copy_frame(_:u64,_:u64){}}
mod pci_command{pub fn quiesce(_: &crate::capability::ProtectionDomain){}}
mod iommu{pub fn revoke(_:u32){}}
mod console{pub fn stop(_:std::fmt::Arguments)->!{panic!("unexpected desktop stop")}}
#[path="../kernel/src/service_restart.rs"]mod service_restart;

mod core_local{pub static CALLER:std::sync::atomic::AtomicU32=std::sync::atomic::AtomicU32::new(12);pub struct CoreLocal;impl CoreLocal{pub fn get()->Self{Self}pub fn current_pd(&self)->u32{CALLER.load(crate::Ordering::Acquire)}}}
mod interrupts{pub struct SyscallRegs{pub rdi:u64,pub rsi:u64,pub rdx:u64}pub static TICKS:core::sync::atomic::AtomicU64=core::sync::atomic::AtomicU64::new(0);}
mod peer_envelope{include!(env!("SEXOS_PEER_ENVELOPE_SOURCE"));}
fn peer_call(caller:u32,slot:u64,buf:u64)->(u64,u64,u64){core_local::CALLER.store(caller,Ordering::Release);let mut r=interrupts::SyscallRegs{rdi:wire::OP_RESTARTS,rsi:slot,rdx:buf};let status=peer_envelope::syscall(&mut r);(status,r.rsi,r.rdx)}
