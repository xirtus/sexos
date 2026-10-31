// SPDX-License-Identifier: GPL-3.0-only
//! Complete app resource runtime; registry, boot mapper and TLB are explicit doubles.
#![allow(dead_code)]
extern crate self as spin;extern crate self as x86_64;extern crate self as sex_pdx;
use std::sync::atomic::{AtomicPtr,AtomicBool,AtomicU64,AtomicUsize,AtomicU32,Ordering};
use std::collections::{BTreeMap,VecDeque};
pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T>{pub const fn new(v:T)->Self{Self(std::sync::Mutex::new(v))}pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>>{self.0.try_lock().ok()}}
#[derive(Clone,Copy)]pub struct VirtAddr(u64);impl VirtAddr{pub fn new(v:u64)->Self{Self(v)}}
pub mod structures{pub mod paging{
 #[derive(Clone,Copy)]pub struct PageTableFlags;
 impl PageTableFlags{pub const PRESENT:Self=Self;pub const USER_ACCESSIBLE:Self=Self;pub const WRITABLE:Self=Self;pub const NO_EXECUTE:Self=Self;}
 impl core::ops::BitOr for PageTableFlags{type Output=Self;fn bitor(self,_:Self)->Self{Self}}
}}
pub mod instructions{pub mod tlb{pub fn flush(va:crate::VirtAddr){crate::FLUSHES.lock().push(va.0);}}}
static FLUSHES:Mutex<Vec<u64>>=Mutex::new(Vec::new());
static LEAVES:Mutex<BTreeMap<u64,usize>>=Mutex::new(BTreeMap::new());
static REGISTRY:[AtomicPtr<capability::ProtectionDomain>;32]=[const{AtomicPtr::new(core::ptr::null_mut())};32];
static WAKES:AtomicUsize=AtomicUsize::new(0);
#[macro_export]macro_rules! serial_println{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}};}
#[path="../kernel/src/app_resource_state.rs"]mod app_resource_state;
#[path="../kernel/src/capability_slot.rs"]mod capability_slot;
#[path="../kernel/src/task_ownership.rs"]pub mod ownership;
#[path="../kernel/src/ipc_ring.rs"]mod ring;
#[path="../kernel/src/ipc/messages.rs"]pub mod messages;
#[path="../kernel/src/ipc/reply_credit.rs"]pub mod reply_credit;
#[path="../kernel/src/capability_table.rs"]pub mod capability_table;
extern crate alloc;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
mod scheduler{
 pub use crate::ownership as task_ownership;
 pub struct Context{pub pd_id:u64,pub pd_ptr:*const crate::capability::ProtectionDomain,pub rip:u64}
 pub struct Task{pub id:u32,pub context:Context,pub state:crate::ownership::TaskOwnership}
 pub struct Scheduler;pub static SCHEDULERS:[Scheduler;1]=[Scheduler];
 impl Scheduler{pub fn activate_task(&self,t:*mut Task)->bool{unsafe{if !crate::app_resources::activation_permitted((*t).context.pd_id as u32){return false}(*t).state.activate()}}}
 pub fn unpark_thread(task:*mut Task){if !task.is_null(){unsafe{(*task).state.wake();}crate::WAKES.fetch_add(1,crate::Ordering::Relaxed);}}
}
mod core_local{pub static INITIALIZED_CORES:crate::AtomicU32=crate::AtomicU32::new(1);pub struct CoreLocal{pub core_id:u32}impl CoreLocal{pub fn get()->Self{Self{core_id:0}}}}
mod interrupts{pub static TICKS:crate::AtomicU64=crate::AtomicU64::new(0);}
mod capability{
 use crate::*;
 #[derive(Clone,Copy)]pub enum CapabilityData{AppLifecycle(app_resource_state::LifecycleGrant),AppActivation(app_resource_state::ActivationGrant),Other}
 #[derive(Clone,Copy)]pub struct Capability{pub id:u32,pub data:CapabilityData,pub cheri_meta:u64}
 pub struct ProtectionDomain{pub id:u32,pub pku_key:u8,pub main_task:AtomicPtr<scheduler::Task>,pub trampoline_task:AtomicPtr<scheduler::Task>,pub message_ring:*mut ring::RingBuffer<messages::MessageType,256>,pub signal_ring:*mut ring::RingBuffer<u8,32>,pub incoming_replies:Mutex<VecDeque<messages::IpcReply>>,pub reply_credits:Mutex<reply_credit::ReplyCredits>,pub cap_table:*mut capability_table::CapabilityTable,caps:Mutex<Vec<(u32,CapabilityData)>>}
 impl ProtectionDomain{
  pub fn find_capability(&self,slot:u32)->Option<Capability>{self.caps.lock().iter().find(|(s,_)|*s==slot).map(|(s,data)|Capability{id:*s,data:*data,cheri_meta:0})}
  pub fn grant_capability(&self,slot:u64,data:CapabilityData){self.caps.lock().push((slot as u32,data));}
  pub fn new(id:u32)->&'static Self{Self::new_key(id,id as u8)}
  pub fn new_key(id:u32,key:u8)->&'static Self{
   let task=Box::into_raw(Box::new(scheduler::Task{id,context:scheduler::Context{pd_id:id as u64,pd_ptr:core::ptr::null(),rip:0x4000},state:ownership::TaskOwnership::new()}));
   let tramp=Box::into_raw(Box::new(scheduler::Task{id:id|0x8000,context:scheduler::Context{pd_id:id as u64,pd_ptr:core::ptr::null(),rip:0},state:ownership::TaskOwnership::new()}));
   let pd=Box::leak(Box::new(Self{id,pku_key:key,main_task:AtomicPtr::new(task),trampoline_task:AtomicPtr::new(tramp),message_ring:Box::into_raw(Box::new(ring::RingBuffer::new())),signal_ring:Box::into_raw(Box::new(ring::RingBuffer::new())),incoming_replies:Mutex::new(VecDeque::new()),reply_credits:Mutex::new(reply_credit::ReplyCredits::new()),cap_table:Box::into_raw(Box::new(capability_table::CapabilityTable::new())),caps:Mutex::new(Vec::new())}));
   unsafe{(*task).context.pd_ptr=pd;(*tramp).context.pd_ptr=pd;}
   REGISTRY[id as usize].store(pd,Ordering::Release);pd
  }
 }
}
mod ipc{
 pub use crate::messages;pub const MAX_DOMAINS:usize=1024;
 pub enum BootPhase{RegistryBuild=1}pub static BOOT_PHASE:core::sync::atomic::AtomicU8=core::sync::atomic::AtomicU8::new(1);
 pub struct Registry;pub static DOMAIN_REGISTRY:Registry=Registry;
 impl Registry{pub fn get(&self,id:u32)->Option<&'static crate::capability::ProtectionDomain>{crate::REGISTRY.get(id as usize).and_then(|p|unsafe{p.load(crate::Ordering::Acquire).as_ref()})}}
}
mod boot_memory{pub fn existing_leaf_address(va:u64)->Option<u64>{crate::LEAVES.lock().get(&va).map(|p|*p as u64)}}
mod memory{
 pub mod va_allocator{pub fn allocate_va(_:usize)->Option<u64>{Some(0x9000_0000)}}
 pub mod manager{
  pub struct Vas;pub static GLOBAL_VAS:crate::Mutex<Option<Vas>>=crate::Mutex::new(Some(Vas));
  impl Vas{pub fn map_physical_range(&mut self,_:crate::VirtAddr,_:u64,_:u64,_:crate::structures::paging::PageTableFlags,_:u8)->Result<(),()>{Ok(())}}
 }
}
mod paging{
 pub static CLOSED:crate::AtomicBool=crate::AtomicBool::new(false);
 pub fn target_cleanup_complete(_:u32)->Result<bool,u64>{Ok(CLOSED.load(crate::Ordering::Acquire))}
}
mod elf{
 pub const PT_LOAD:u32=1;
 #[repr(C)]#[derive(Clone,Copy)]pub struct ElfHeader{pub phoff:u64,pub phentsize:u16,pub phnum:u16}
 #[repr(C)]#[derive(Clone,Copy)]pub struct ProgramHeader{pub p_type:u32,pub p_vaddr:u64,pub p_memsz:u64}
}
#[path="../kernel/src/app_resources.rs"]mod app_resources;
fn fixture_elf()->Vec<u8>{
 let h=elf::ElfHeader{phoff:16,phentsize:core::mem::size_of::<elf::ProgramHeader>()as u16,phnum:1};
 let ph=elf::ProgramHeader{p_type:1,p_vaddr:0x1000,p_memsz:4096};
 let mut bytes=vec![0;16+core::mem::size_of::<elf::ProgramHeader>()];
 unsafe{core::ptr::write_unaligned(bytes.as_mut_ptr()as *mut elf::ElfHeader,h);core::ptr::write_unaligned(bytes.as_mut_ptr().add(16)as *mut elf::ProgramHeader,ph);}bytes
}
#[test]fn actual_lifecycle_checks_all_barriers_retries_hints_then_hands_off_once(){
 use app_resource_state::*;use capability::*;use ownership::{Admission,Phase};
 let services=[3,1,11,13,10];for id in services{ProtectionDomain::new(id);}
 let pager=ProtectionDomain::new(14);let target=ProtectionDomain::new(15);let next=ProtectionDomain::new_key(16,15);
 let mut next_leaves=Box::new([0u64;17]);
 for i in 0..17{let va=if i==0{0x5000_0000}else{0x7101_0000-65536+(i as u64-1)*4096};next_leaves[i]=(0x20000+i as u64*4096)|5|(15u64<<59);LEAVES.lock().insert(va,&mut next_leaves[i]as *mut u64 as usize);}
 // Mock enforcement key is independent of registry identity.
 let base=0x4000_0000;let top=0x7001_0000;let mut leaves=Box::new([0u64;17]);
 for i in 0..17{let va=if i==0{base}else{top-65536+(i as u64-1)*4096};leaves[i]=(0x1000+i as u64*4096)|5|(15u64<<59);LEAVES.lock().insert(va,&mut leaves[i]as *mut u64 as usize);}
 let spindle=ProtectionDomain::new_key(12,12);let spindle_next=ProtectionDomain::new_key(17,12);
 let mut spindle_leaves=Box::new([0u64;17]);let mut spindle_next_leaves=Box::new([0u64;17]);
 for (base,top,pa,leaves) in [(0x6000_0000,0x7201_0000,0x40000,&mut spindle_leaves),(0x6100_0000,0x7301_0000,0x60000,&mut spindle_next_leaves)]{
  for i in 0..17{let va=if i==0{base}else{top-65536+(i as u64-1)*4096};leaves[i]=(pa+i as u64*4096)|5|(12u64<<59);LEAVES.lock().insert(va,&mut leaves[i]as *mut u64 as usize);}
 }
 let third=ProtectionDomain::new_key(18,15);let mut third_leaves=Box::new([0u64;17]);
 for i in 0..17{let va=if i==0{0x6200_0000}else{0x7401_0000-65536+(i as u64-1)*4096};third_leaves[i]=(0x80000+i as u64*4096)|5|(15u64<<59);LEAVES.lock().insert(va,&mut third_leaves[i]as *mut u64 as usize);}
 app_resources::plan_boot_apps(&[15,16,12,17,18]);app_resources::capture_boot_image(15,15,&fixture_elf(),base,top).unwrap();
 app_resources::capture_boot_image(16,15,&fixture_elf(),0x5000_0000,0x7101_0000).unwrap();
 app_resources::capture_boot_image(12,12,&fixture_elf(),0x6000_0000,0x7201_0000).unwrap();
 app_resources::capture_boot_image(17,12,&fixture_elf(),0x6100_0000,0x7301_0000).unwrap();
 app_resources::capture_boot_image(18,15,&fixture_elf(),0x6200_0000,0x7401_0000).unwrap();
 app_resources::init(14,3,&[(15,300),(16,301),(12,154),(17,302),(18,307)],services);
 app_resources::reserve_successor(15,16,3,44);
 app_resources::reserve_successor(16,18,3,55);
 app_resources::reserve_successor(12,17,3,49);
 let reserved=*next_leaves;assert_eq!(reserved,[0;17]);assert!(!app_resources::activation_permitted(16));assert!(!app_resources::accepts_ipc(16));
 FLUSHES.lock().clear();
 let shell=ipc::DOMAIN_REGISTRY.get(3).unwrap();
 assert_eq!(app_resources::activation(shell,44,0),[0,16,16,301,0]);
 assert_eq!(app_resources::activation(shell,44,1),[RETRY,0,0,0,0]);
 assert_eq!(app_resources::activation(pager,44,1),[INVALID,0,0,0,0]);
 assert_eq!(app_resources::activation(shell,0x1_0000_002c,1),[INVALID,0,0,0,0]);
 assert_eq!(app_resources::status(shell,38,1),[0,15,15,300,3]);
 assert_eq!(app_resources::status(shell,38,0),[LIVE,0,0,0,0]);
 assert_eq!(app_resources::status(shell,0x1_0000_0026,1),[INVALID,0,0,0,0]);
 assert_eq!(app_resources::ack(shell,38,15),LIVE);assert_eq!(app_resources::retire(pager,38),[LIVE,0,0,0]);
 assert_eq!(app_resources::retire(shell,38),[DENIED,0,0,0]);
 let task=unsafe{&*target.main_task.load(Ordering::Acquire)};
 assert!(task.state.activate());assert_eq!(task.state.try_admit(||Ok::<(),()>(())),Admission::Published);assert!(task.state.begin_dispatch(0));assert!(task.state.attach(0));
 task.state.exit();assert_eq!(app_resources::status(shell,38,0),[RETRY,0,0,0,0]);app_resources::poll_exit_notifications();assert!(app_resources::accepts_ipc(15));
 assert!(task.state.begin_switch_out(0));unsafe{assert!(task.state.release_after_stack_switch(0));}assert_eq!(task.state.snapshot().phase,Phase::Dead);
 // Simulate a saturated consumer. Retry survives both full queue and accepted
 // notice discarded by a nested wait; ACK alone stops periodic notifications.
 for _ in 0..256{unsafe{(*shell.message_ring).enqueue(messages::MessageType::Empty).unwrap();}}
 app_resources::poll_exit_notifications();assert!(!app_resources::accepts_ipc(15));assert!(app_resources::accepts_ipc(9));
 while unsafe{(*shell.message_ring).dequeue()}.is_some(){}
 interrupts::TICKS.store(15,Ordering::Release);app_resources::poll_exit_notifications();assert!(unsafe{(*shell.message_ring).dequeue()}.is_none());
 interrupts::TICKS.store(16,Ordering::Release);app_resources::poll_exit_notifications();assert!(matches!(unsafe{(*shell.message_ring).dequeue()},Some(messages::MessageType::IpcCall{func_id:0x107,arg0:15,arg1:15,caller_pd:0,..})));
 interrupts::TICKS.store(32,Ordering::Release);app_resources::poll_exit_notifications();assert!(unsafe{(*shell.message_ring).dequeue()}.is_some());
 assert_eq!(app_resources::ack(shell,38,0x1_0000_000f),DENIED);
 assert_eq!(app_resources::ack(shell,38,15),0);assert_eq!(app_resources::ack(shell,38,15),0);
 interrupts::TICKS.store(48,Ordering::Release);app_resources::poll_exit_notifications();assert!(unsafe{(*shell.message_ring).dequeue()}.is_none());
 assert_eq!(app_resources::status(shell,38,0),[0,15,15,31,1]);assert_eq!(app_resources::retire(pager,38),[RETRY,0,0,0]);
 for id in &services[1..]{assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(*id).unwrap(),38,15),0);}
 assert_eq!(app_resources::status(shell,38,0),[0,15,15,31,31]);assert_eq!(app_resources::retire(pager,38),[RETRY,0,0,0]);
 paging::CLOSED.store(true,Ordering::Release);let before=*leaves;leaves[16]^=2;
 assert_eq!(app_resources::retire(pager,38),[DENIED,0,0,0]);assert_eq!(leaves[0],before[0]);assert!(FLUSHES.lock().is_empty());leaves[16]=before[16]|0x60;
 unsafe{(*target.message_ring).enqueue(messages::MessageType::Empty).unwrap();(*target.signal_ring).enqueue(3).unwrap();}target.incoming_replies.lock().push_back(messages::IpcReply{value:7,caller_pd:13,caller_task:213});assert_eq!(target.reply_credits.lock().grant(13),reply_credit::Grant::Added);
 let held=unsafe{&*target.cap_table};for _ in 0..3{held.insert(capability::CapabilityData::Other);}held.seal();assert!(!held.holds_none(|_|true));
 let result=[0,0x9000_0000,17*4096,15];assert_eq!(app_resources::retire(pager,38),result);assert_eq!(*leaves,[0;17]);assert_eq!(FLUSHES.lock().len(),17);
 assert_eq!(unsafe{(*target.message_ring).len()},0);assert_eq!(unsafe{(*target.signal_ring).len()},0);assert!(target.incoming_replies.lock().is_empty());assert_eq!(target.reply_credits.lock().outstanding(),0);
 assert!(held.holds_none(|_|true));for id in 1..=4{assert!(held.find(id).is_none());}
 assert_eq!(app_resources::retire(pager,38),result);assert_eq!(FLUSHES.lock().len(),17);
 assert_eq!(ipc::DOMAIN_REGISTRY.get(15).unwrap().main_task.load(Ordering::Acquire),target.main_task.load(Ordering::Acquire));
 assert_eq!(app_resources::activation(shell,44,0),[0,16,16,301,1]);
 core_local::INITIALIZED_CORES.store(2,Ordering::Release);
 assert_eq!(app_resources::activation(shell,44,1),[DENIED,0,0,0,0]);assert_eq!(*next_leaves,[0;17]);
 core_local::INITIALIZED_CORES.store(1,Ordering::Release);
 let next_task=next.main_task.load(Ordering::Acquire);
 unsafe{(*next_task).context.pd_id=15;}
 assert_eq!(app_resources::activation(shell,44,1),[DENIED,0,0,0,0]);assert_eq!(*next_leaves,[0;17]);
 unsafe{(*next_task).context.pd_id=16;}
 next_leaves[16]=1;let before=FLUSHES.lock().len();
 assert_eq!(app_resources::activation(shell,44,1),[DENIED,0,0,0,0]);assert_eq!(next_leaves[0],0);assert_eq!(FLUSHES.lock().len(),before);next_leaves[16]=0;
 assert_eq!(app_resources::activation(shell,44,1),[0,16,16,301,2]);
 assert!(next_leaves.iter().all(|p|*p!=0));assert!(app_resources::accepts_ipc(16));assert!(!app_resources::accepts_ipc(15));
 assert_eq!(unsafe{(*next_task).state.snapshot().phase},Phase::Detached);
 let before=FLUSHES.lock().len();assert_eq!(app_resources::activation(shell,44,1),[0,16,16,301,2]);assert_eq!(FLUSHES.lock().len(),before);
 // Old task custody is still permanently dead; receipt replay after successor
 // death reports exhaustion and cannot resurrect either identity.
 unsafe{(*next_task).state.exit();}app_resources::poll_exit_notifications();
 assert_eq!(app_resources::activation(shell,44,1),[0,16,16,301,3]);
 assert_eq!(unsafe{(*next_task).state.snapshot().phase},Phase::Dead);assert_eq!(task.state.snapshot().phase,Phase::Dead);

 // A third subject cannot skip second-incarnation cleanup, reuse its leaves,
 // or turn either consumed receipt back into an activation.
 assert_eq!(app_resources::activation(shell,55,0),[0,18,18,307,0]);
 assert_eq!(app_resources::activation(shell,55,1),[RETRY,0,0,0,0]);assert_eq!(*third_leaves,[0;17]);
 for id in &services[..4]{assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(*id).unwrap(),39,16),0);}
 assert_eq!(app_resources::retire(pager,39)[0],RETRY);
 assert_eq!(app_resources::activation(shell,55,1),[RETRY,0,0,0,0]);
 assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(services[4]).unwrap(),39,16),0);
 assert_eq!(app_resources::retire(pager,39)[0],0);assert_eq!(*next_leaves,[0;17]);
 assert_eq!(app_resources::activation(shell,55,0),[0,18,18,307,1]);
 assert_eq!(app_resources::activation(shell,55,1),[0,18,18,307,2]);
 assert!(third_leaves.iter().all(|p|*p!=0));assert!(!app_resources::accepts_ipc(16));assert!(app_resources::accepts_ipc(18));
 let task18=third.main_task.load(Ordering::Acquire);unsafe{(*task18).state.exit();}app_resources::poll_exit_notifications();
 for id in services{assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(id).unwrap(),50,18),0);}
 assert_eq!(app_resources::retire(pager,50)[0],0);assert_eq!(*third_leaves,[0;17]);
 let before=FLUSHES.lock().len();
 assert_eq!(app_resources::activation(shell,44,1),[0,16,16,301,3]);
 assert_eq!(app_resources::activation(shell,55,1),[0,18,18,307,3]);
 assert_eq!(FLUSHES.lock().len(),before);assert!(!app_resources::accepts_ipc(18));

 // A second logical app consumes only its own reserved identity/grant, even
 // after the first pair has exhausted its receipt.
 assert_eq!(app_resources::activation(shell,49,0),[0,17,17,302,0]);
 assert_eq!(app_resources::activation(shell,49,1),[RETRY,0,0,0,0]);assert_eq!(*spindle_next_leaves,[0;17]);
 let spindle_task=spindle.main_task.load(Ordering::Acquire);unsafe{(*spindle_task).state.exit();}app_resources::poll_exit_notifications();
 for id in services{assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(id).unwrap(),40,12),0);}
 assert_eq!(app_resources::retire(pager,40)[0],0);assert_eq!(*spindle_leaves,[0;17]);
 assert_eq!(app_resources::activation(shell,49,0),[0,17,17,302,1]);
 assert_eq!(app_resources::activation(shell,44,1),[0,16,16,301,3]);assert_eq!(*spindle_next_leaves,[0;17]);
 assert_eq!(app_resources::activation(shell,49,1),[0,17,17,302,2]);let before=FLUSHES.lock().len();
 assert_eq!(app_resources::activation(shell,49,1),[0,17,17,302,2]);assert_eq!(FLUSHES.lock().len(),before);
 assert!(app_resources::accepts_ipc(17));assert!(!app_resources::accepts_ipc(12));assert!(!app_resources::accepts_ipc(16));
 let task17=spindle_next.main_task.load(Ordering::Acquire);unsafe{(*task17).state.exit();}app_resources::poll_exit_notifications();
 for id in services{assert_eq!(app_resources::ack(ipc::DOMAIN_REGISTRY.get(id).unwrap(),45,17),0);}
 assert_eq!(app_resources::retire(pager,45)[0],0);assert_eq!(*spindle_next_leaves,[0;17]);
 assert_eq!(app_resources::activation(shell,49,1),[0,17,17,302,3]);

}
