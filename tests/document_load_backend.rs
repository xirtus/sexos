//! Actual VFS load/save/cancel adapter and journal; metadata/transport are mocks.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
pub const BLOCK_MAX_XFER:u64=4096;
#[path="../crates/sex-pdx/src/document.rs"]pub mod wire;
#[path="../servers/sexfiles/src/document.rs"]mod service;
mod document{pub use crate::wire::*;pub use crate::service::*;}
#[path="../servers/sexfiles/src/document_journal.rs"]mod document_journal;
// The production modules refer to the ABI as sex_pdx::document.
// Re-export its constants alongside the actual service API for this harness.
use std::sync::{Mutex,atomic::{AtomicU64,Ordering}};
use document::Backend;
struct Model{bytes:Vec<u8>,reads:Vec<(u64,usize)>,writes:usize,gen:u16,size:u32,growth:usize}
static MODEL:Mutex<Option<Model>>=Mutex::new(None);
static TEST_LOCK:Mutex<()>=Mutex::new(());
struct Entry{size_bytes:u32,gen:u16,name:[u8;24]}
fn diskfs_bridge_get_buf_va()->u64{0x400000}
fn v4_ensure(_:u64)->Result<(),u32>{Ok(())}
fn v4_get(id:u64)->Result<Entry,u32>{
 let m=MODEL.lock().unwrap();let m=m.as_ref().unwrap();
 if id==3{Ok(Entry{size_bytes:m.size,gen:m.gen,name:[0;24]})}else{Err(1)}
}
fn handle_diskfs_create(_:u64,_:u64,_:u64)->u64{5}
struct DocumentGrowth;
impl DocumentGrowth{
 fn new(_:usize,_:u64)->Result<Self,u32>{MODEL.lock().unwrap().as_mut().unwrap().growth+=1;Ok(Self)}
 fn step(&mut self,_:u64)->Result<bool,u32>{Ok(true)}fn cancel(&self){}
}
pub struct DocumentBackend;
struct DocumentIo{id:u8}
impl document_journal::Io for DocumentIo{
 fn read(&mut self,offset:u64,out:&mut[u8])->Result<(),u32>{
  assert_eq!(self.id,3);let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();m.reads.push((offset,out.len()));
  out.copy_from_slice(&m.bytes[offset as usize..offset as usize+out.len()]);Ok(())
 }
 fn write(&mut self,offset:u64,bytes:&[u8])->Result<(),u32>{
  let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();m.writes+=1;m.bytes[offset as usize..offset as usize+bytes.len()].copy_from_slice(bytes);Ok(())
 }
 fn sync(&mut self)->Result<(),u32>{Ok(())}
}
fn document_read(id:u8,offset:u64,out:&mut[u8])->Result<(),u32>{use document_journal::Io;DocumentIo{id}.read(offset,out)}
include!(env!("DOCUMENT_LOAD_BACKEND_SOURCE"));
fn setup(){
 unsafe{DOCUMENT_JOB=None;DOCUMENT_LOAD_JOB=None;}DOCUMENT_WRITE_LOCK.store(0,Ordering::Relaxed);
 *MODEL.lock().unwrap()=Some(Model{bytes:vec![0;65536],reads:vec![],writes:0,gen:1,size:65536,growth:0});
 document_journal::save(&mut DocumentIo{id:3},3,0x40001,b"durable",&mut[0;wire::MAX]).unwrap();
 let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();m.reads.clear();m.writes=0;
}
#[test]fn load_holds_exact_object_custody_until_owner_completion_or_cancel(){
 let _g=TEST_LOCK.lock().unwrap();
 for caller in [9,16,18]{
  setup();let mut b=DocumentBackend;let mut out=[0;wire::MAX];assert_eq!(b.load_step(caller,3,&mut out),Ok(None));
  assert!(document_locked(3));assert!(!document_locked(4));assert!(!document_locked(u64::MAX));
  let other=if caller==9{16}else{9};
  assert_eq!(b.load_step(other,3,&mut[0;wire::MAX]),Err(wire::BUSY));assert_eq!(b.save_step(other,3,b"mutation"),Err(wire::BUSY));
  b.cancel_load(other);b.cancel_save(caller);assert!(document_locked(3));
  let mut completed=false;for _ in 0..100{if let Some(n)=b.load_step(caller,3,&mut out).unwrap(){assert_eq!(n,7);completed=true;break}assert!(document_locked(3));}
  assert!(completed);assert_eq!(&out[..7],b"durable");assert!(document_locked(3));b.cancel_load(caller);assert!(!document_locked(3));
  let m=MODEL.lock().unwrap();let m=m.as_ref().unwrap();assert_eq!(m.writes,0);assert_eq!(m.growth,0);assert!(m.reads.iter().all(|r|r.1<=4096));
 }
}
#[test]fn save_custody_rejects_load_without_stealing_owner_lock(){
 let _g=TEST_LOCK.lock().unwrap();setup();let mut b=DocumentBackend;
 assert_eq!(b.save_step(9,3,b"new"),Ok(false));let before=MODEL.lock().unwrap().as_ref().unwrap().reads.len();
 assert_eq!(b.load_step(16,3,&mut[0;wire::MAX]),Err(wire::BUSY));b.cancel_load(9);assert!(document_locked(3));
 assert_eq!(MODEL.lock().unwrap().as_ref().unwrap().reads.len(),before);b.cancel_save(9);assert!(!document_locked(3));
 assert_eq!(b.load_step(16,3,&mut[0;wire::MAX]),Ok(None));b.cancel_load(16);
}
#[test]fn terminal_service_cleanup_scrubs_and_releases_actual_backend_load(){
 let _g=TEST_LOCK.lock().unwrap();setup();let mut b=DocumentBackend;let mut s=document::Service::new();
 let r=wire::Request{seq:1,kind:wire::LOAD,object:3,offset:0,a:0x40001,b:0};
 s.handle(18,true,r,&mut b);s.progress(&mut b);assert!(document_locked(3));let before=MODEL.lock().unwrap().as_ref().unwrap().reads.len();
 s.retire(18,&mut b);s.progress(&mut b);s.poll(|_,_|panic!("retired completion"));assert!(!document_locked(3));
 assert_eq!(MODEL.lock().unwrap().as_ref().unwrap().reads.len(),before);assert!(!s.pending());
 assert_eq!(wire::decode_reply(s.handle(18,true,r,&mut b).unwrap(),1),Some(Err(wire::DENIED)));
}
