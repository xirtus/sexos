// SPDX-License-Identifier: GPL-3.0-only
//! Actual Display server + atomic bank, synthetic held syscall results only.
//! No BOOT/PKU/native publication claim follows from this host fixture.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
#[path="../crates/sex-pdx/src/desktop_recording.rs"]pub mod desktop_recording;
#[path="../crates/sex-pdx/src/desktop_recording_capture.rs"]pub mod desktop_recording_capture;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../crates/sex-pdx/src/desktop_look.rs"]pub mod desktop_look;
#[path="../servers/sexdisplay/src/recording_capture.rs"]mod recording_capture;
pub use memory_region::MemoryRegionInfo;
use desktop_recording_capture as c;
use desktop_recording as r;
use std::sync::Mutex;
#[derive(Clone,Copy)]pub struct PdxMessage {pub caller_pd:u32,pub caller_task:u32,pub arg0:u64,pub arg1:u64,pub arg2:u64}
struct Fixture {base:usize,epoch:u64,shell:bool,record:bool,spindle:bool,blocked:bool,replies:Vec<(u32,u64)>,events:Vec<[u64;3]>}
static FIX:Mutex<Fixture>=Mutex::new(Fixture{base:0,epoch:1,shell:true,record:true,spindle:true,blocked:false,replies:Vec::new(),events:Vec::new()});
pub mod app_catalog {
 pub struct Row {pub lifecycle_slot:usize}
 pub fn by_target(target:u32)->Option<Row>{match target{42|12=>Some(Row{lifecycle_slot:target as usize}),_=>None}}
}
pub mod app_lifecycle {
 pub const LIVE:u64=5;
 #[derive(Debug,PartialEq)]pub struct Info;
 pub struct Catalog {pub target_pd:u32,pub task_id:u32,pub coordinator_pd:u32}
 pub fn catalog(slot:usize)->Result<Catalog,u64>{Ok(Catalog{target_pd:slot as u32,task_id:if slot==42{107}else{12},coordinator_pd:3})}
 pub fn info(slot:usize)->Result<Info,u64>{let f=super::FIX.lock().unwrap();if if slot==42{f.record}else{f.spindle}{Err(LIVE)}else{Ok(Info)}}
}
pub mod crash_log {
 pub struct Peer {pub epoch:u64,pub enabled:bool}
 impl Peer {pub fn live(&self)->bool{self.enabled}}
 pub fn peer_state(slot:u64)->Result<Peer,u64>{assert_eq!(slot,6);let f=super::FIX.lock().unwrap();Ok(Peer{epoch:f.epoch,enabled:f.shell})}
}
pub fn memory_region_info(slot:u64)->Result<MemoryRegionInfo,u64>{assert_eq!(slot,c::SLOT);Ok(MemoryRegionInfo{region_id:c::REGION_ID,length:c::BYTES as u64,rights:7,peer_pd:Some(42)})}
pub fn memory_region_view(slot:u64,offset:u64,len:u64,rights:u8)->Result<memory_region::RegionView,u64>{
 assert_eq!((slot,offset,len,rights),(95,0,c::BYTES as u64,7));Ok(memory_region::RegionView{region_id:c::REGION_ID,virtual_address:FIX.lock().unwrap().base as u64,length:len,dma_address:None})
}
pub fn pdx_reply(target:u32,word:u64)->u64{let mut f=FIX.lock().unwrap();if f.blocked{return 1;}f.replies.push((target,word));0}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){assert_eq!((slot,op),(6,c::OP_EVENT));let mut f=FIX.lock().unwrap();if f.blocked{return(1,0);}f.events.push([a,b,c]);(0,0)}
struct Backing(*mut u8);
impl Drop for Backing{fn drop(&mut self){unsafe{std::alloc::dealloc(self.0,std::alloc::Layout::from_size_align(c::BYTES,4096).unwrap())}}}
fn setup()->(Backing,recording_capture::Server,c::Bank){
 assert!(r::ENABLED,"compile this fixture with SEXOS_RECORDER=1");
 let p=unsafe{std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(c::BYTES,4096).unwrap())};assert!(!p.is_null());
 *FIX.lock().unwrap()=Fixture{base:p as usize,epoch:1,shell:true,record:true,spindle:true,blocked:false,replies:Vec::new(),events:Vec::new()};
 let mut s=recording_capture::Server::new();s.attach(1);
 let info=MemoryRegionInfo{region_id:c::REGION_ID,length:c::BYTES as u64,rights:3,peer_pd:Some(1)};
 let view=memory_region::RegionView{region_id:c::REGION_ID,virtual_address:p as u64,length:c::BYTES as u64,dma_address:None};
 (Backing(p),s,unsafe{c::Bank::from_resolved(info,view,false).unwrap()})
}
fn scope()->c::Scope{c::Scope{session:r::SessionSequence::new(1,1).unwrap(),shell_epoch:1,display_epoch:1,record_task:107,spindle_task:12}}
fn seq(n:u32)->r::RoleSequence{r::RoleSequence::new(1,n).unwrap()}
fn control(kind:c::ControlKind,n:u32)->PdxMessage{let a=c::ControlRequest{kind,sequence:seq(n),scope:scope()}.args().unwrap();PdxMessage{caller_pd:if kind==c::ControlKind::Acquire{42}else{3},caller_task:if kind==c::ControlKind::Acquire{107}else{3},arg0:a[0],arg1:a[1],arg2:a[2]}}
fn ack(kind:c::AckKind,n:u32,generation:u64)->PdxMessage{let a=c::FrameAck{kind,sequence:seq(n),session:scope().session,generation,display_epoch:1}.args().unwrap();PdxMessage{caller_pd:42,caller_task:107,arg0:a[0],arg1:a[1],arg2:a[2]}}
fn front()->Vec<u32>{(0..c::PIXELS).map(|i|i as u32).collect()}
fn arm(s:&mut recording_capture::Server){s.control(control(c::ControlKind::Arm,1),100,true);let mark=s.mark();s.published(mark,true,1,640,360,&front(),0,100);s.poll(100);}
fn last_frame()->c::FrameReply{let f=FIX.lock().unwrap();let (_,word)=f.replies.iter().rev().find(|(t,_)|*t==42).unwrap();c::FrameReply::decode(1,*word).unwrap()}
#[test]fn actual_sender_scope_and_visible_publication_gate_arm(){
 let(_b,mut s,_reader)=setup();let mut wrong=control(c::ControlKind::Arm,1);wrong.caller_task=4;s.control(wrong,100,true);assert!(s.mark().is_none());
 s.control(control(c::ControlKind::Arm,1),100,true);let mark=s.mark();assert!(mark.is_some());
 let mut stale=mark.unwrap();stale[0]+=1;s.published(Some(stale),true,1,640,360,&front(),0,101);s.poll(101);
 assert!(!FIX.lock().unwrap().replies.iter().any(|(_,w)|c::ControlReply::decode(1,*w).is_some_and(|r|r.status==c::ControlStatus::Armed)));
 s.published(mark,true,3,640,360,&front(),0,103);s.poll(103);
 let f=FIX.lock().unwrap();let reply=c::ControlReply::decode(1,f.replies.last().unwrap().1).unwrap();assert_eq!(reply.status,c::ControlStatus::Armed);assert!(reply.matches(seq(1),107));
}
#[test]fn retained_ready_requires_delivery_and_old_ack_cannot_displace_new_frame(){
 let(_b,mut s,reader)=setup();arm(&mut s);
 s.control(control(c::ControlKind::Acquire,0x80000),110,true);
 FIX.lock().unwrap().blocked=true;s.published(s.mark(),true,2,640,360,&front(),0,110);s.poll(110);
 assert_eq!(reader.snapshot().unwrap().header.generation,2);
 s.frame(ack(c::AckKind::Frame,0x80001,2),111);assert!(reader.snapshot().is_some());
 FIX.lock().unwrap().blocked=false;s.poll(112);assert_eq!(last_frame().status,c::FrameStatus::Ready);
 let old_ack=ack(c::AckKind::Frame,0x80001,2);s.frame(old_ack,113);s.poll(113);assert_eq!(last_frame().status,c::FrameStatus::Acked);assert!(reader.snapshot().is_none());
 s.control(control(c::ControlKind::Acquire,0x80002),114,true);s.frame(old_ack,115);
 assert_eq!(last_frame().status,c::FrameStatus::Denied);assert!(!s.needs_publication(309));
 s.published(s.mark(),true,3,640,360,&front(),0,309);assert!(reader.snapshot().is_none());
 s.published(s.mark(),true,4,640,360,&front(),0,310);
 assert_eq!(reader.snapshot().unwrap().header.sequence,seq(0x80002));
 s.frame(old_ack,311);s.poll(311);let reply=last_frame();assert_eq!(reply.status,c::FrameStatus::Ready);assert!(reply.matches(seq(0x80002),107));
}
#[test]fn stop_scrubs_before_reply_pressure_and_tombstone_blocks_arm_replay(){
 let(_b,mut s,reader)=setup();arm(&mut s);s.control(control(c::ControlKind::Acquire,0x80000),110,true);
 s.published(s.mark(),true,2,640,360,&front(),0,110);assert!(reader.snapshot().is_some());
 FIX.lock().unwrap().blocked=true;let old=s.mark();s.control(control(c::ControlKind::Stop,0x7ffff),111,true);
 assert!(reader.snapshot().is_none());assert!(s.mark().is_none());s.control(control(c::ControlKind::Arm,1),112,true);
 s.published(old,true,3,640,360,&front(),0,113);assert!(reader.snapshot().is_none());
 FIX.lock().unwrap().blocked=false;s.poll(114);
 assert!(!FIX.lock().unwrap().replies.iter().any(|(_,w)|c::FrameReply::decode(1,*w).is_some_and(|r|r.status==c::FrameStatus::Ready)));
 s.frame(ack(c::AckKind::Release,0xfffff,0),115);s.poll(115);assert_eq!(last_frame().status,c::FrameStatus::Released);
 s.frame(ack(c::AckKind::Release,0xfffff,0),116);s.poll(116);assert_eq!(last_frame().status,c::FrameStatus::Released);
}
#[test]fn release_during_pending_indicator_revokes_and_unregistered_release_is_denied(){
 let(_b,mut s,reader)=setup();s.frame(ack(c::AckKind::Release,0xfffff,0),99);assert_eq!(last_frame().status,c::FrameStatus::Denied);
 s.control(control(c::ControlKind::Arm,1),100,true);let old=s.mark();s.frame(ack(c::AckKind::Release,0xfffff,0),101);s.poll(101);
 assert_eq!(last_frame().status,c::FrameStatus::Released);s.published(old,true,1,640,360,&front(),0,102);assert!(reader.snapshot().is_none());assert!(s.mark().is_none());
}
#[test]fn epoch_retirement_and_duration_revoke_held_frames_before_positive_delivery(){
 for reason in 0..3 {
  let(_b,mut s,reader)=setup();arm(&mut s);s.control(control(c::ControlKind::Acquire,0x80000),110,true);s.published(s.mark(),true,2,640,360,&front(),0,110);
  if reason==0{FIX.lock().unwrap().epoch=2;}else if reason==1{FIX.lock().unwrap().record=false;}
  s.poll(if reason==2{60100}else{111});assert!(reader.snapshot().is_none());assert!(s.mark().is_none());
  assert!(!FIX.lock().unwrap().replies.iter().any(|(_,w)|c::FrameReply::decode(1,*w).is_some_and(|r|r.status==c::FrameStatus::Ready)));
 }
}
#[test]fn stop_before_arm_tombstones_scope_and_capture_requires_actual_recorder_task(){
 let(_b,mut s,reader)=setup();s.control(control(c::ControlKind::Stop,0x7ffff),99,true);s.poll(99);
 let word=FIX.lock().unwrap().replies.last().unwrap().1;
 assert_eq!(c::ControlReply::decode(1,word).unwrap().status,c::ControlStatus::Stopped);
 s.control(control(c::ControlKind::Arm,1),100,true);assert!(s.mark().is_none());assert!(reader.snapshot().is_none());
 drop(s);drop(reader);drop(_b);
 let(_b,mut s,reader)=setup();arm(&mut s);
 let mut wrong=control(c::ControlKind::Acquire,0x80000);wrong.caller_task=108;s.control(wrong,110,true);
 s.published(s.mark(),true,2,640,360,&front(),0,110);assert!(reader.snapshot().is_none());
 let mut wrong_scope=scope();wrong_scope.spindle_task=13;
 let a=c::ControlRequest{kind:c::ControlKind::Acquire,sequence:seq(0x80000),scope:wrong_scope}.args().unwrap();
 let wrong=PdxMessage{caller_pd:42,caller_task:107,arg0:a[0],arg1:a[1],arg2:a[2]};s.control(wrong,111,true);
 s.published(s.mark(),true,3,640,360,&front(),0,111);assert!(reader.snapshot().is_none());
}

#[test]fn lost_marker_or_unsupported_placement_immediately_revokes_held_pixels(){
 for missing_pixels in [true,false] {
  let(_b,mut s,reader)=setup();arm(&mut s);s.control(control(c::ControlKind::Acquire,0x80000),110,true);
  s.published(s.mark(),true,2,640,360,&front(),0,110);assert!(reader.snapshot().is_some());
  if missing_pixels{s.published(s.mark(),false,3,640,360,&front(),0,111);}else{s.placement(false);}
  assert!(reader.snapshot().is_none());assert!(s.mark().is_none());
 }
}
