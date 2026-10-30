// SPDX-License-Identifier: GPL-3.0-only
//! Actual Browser DNS client, register ABI, DNS codec and private job machine.
extern crate self as sex_pdx;
extern crate self as sex_net;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/network_dns.rs"]pub mod network_dns;
#[path="../crates/sex-net/src/dns_wire.rs"]pub mod dns_wire;
mod jobs { #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Identity {pub pd:u32,pub task:u32} }
#[path="../servers/sexnet/src/dns_jobs.rs"]mod dns_jobs;
#[path="../apps/kaleidoscope/src/dns_probe.rs"]mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const SLOT_DISPLAY:u64=5;pub const ERR_SERVICE_NOT_READY:u64=9;
#[macro_export]macro_rules! serial_record {($($t:tt)*)=>{let _=std::format!($($t)*);};}
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
struct Peer {begins:Vec<u32>,response:Option<Vec<u8>>,answer:bool,error:Option<u32>}
impl dns_jobs::Wire for Peer {
 fn begin(&mut self,ticket:u32)->Result<(),u32>{
  self.begins.push(ticket);if self.answer{self.response=Some(response(ticket as u16));}
  self.error.map_or(Ok(()),Err)
 }
 fn receive(&mut self,out:&mut[u8;512])->Result<Option<usize>,u32>{
  if let Some(bytes)=self.response.take(){out[..bytes.len()].copy_from_slice(&bytes);Ok(Some(bytes.len()))}else{Ok(None)}
 }
 fn abort(&mut self){self.response=None;}
}
fn response(id:u16)->Vec<u8>{
 let mut bytes=vec![0;34];bytes[..2].copy_from_slice(&id.to_be_bytes());bytes[2..8].copy_from_slice(&[0x81,0x80,0,1,0,1]);
 bytes[12..].copy_from_slice(b"\x05probe\x05sexos\x04test\x00\x00\x01\x00\x01");
 bytes.extend_from_slice(&[0xC0,12,0,1,0,1,0,0,0,60,0,4,10,0,2,2]);bytes
}
struct World {
 machine:dns_jobs::Machine,wire:Peer,request:Option<network_dns::Request>,accepted:Vec<network_dns::Request>,
 calls:usize,full_first:bool,last_full:u32,full_always:bool,display:Vec<(usize,[u8;8])>,display_full:bool,
}
fn reset(full:bool){*WORLD.lock().unwrap()=Some(World {
 machine:dns_jobs::Machine::new(),wire:Peer {begins:Vec::new(),response:None,answer:true,error:None},
 request:None,accepted:Vec::new(),calls:0,full_first:full,last_full:0,full_always:false,
 display:Vec::new(),display_full:false,
});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
 let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();
 if slot==SLOT_DISPLAY {
  assert_eq!(op,0xFB);assert_eq!(a,300);assert_eq!((c>>8)&255,8);
  if w.display_full{return(ERR_SERVICE_NOT_READY,0);}
  w.display.push(((c&255)as usize,b.to_le_bytes()));return(0,0);
 }
 assert_eq!(slot,SLOT_NET);assert_eq!(op,network_dns::OP);
 let r=network_dns::Request::decode([a,b,c]).unwrap();w.calls+=1;
 if w.full_always||(w.full_first&&w.last_full!=r.seq){w.last_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
 assert!(w.request.is_none());assert!(w.accepted.last().is_none_or(|prior|r.seq>prior.seq),"accepted request replayed or sequence regressed");
 w.accepted.push(r);w.request=Some(r);(0,0)
}
fn turn(p:&mut probe::Client,now:u64){
 p.advance(now);
 let reply={let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();
  if let Some(r)=w.request.take(){assert!(w.machine.handle(jobs::Identity {pd:15,task:15},r,now,false,&mut w.wire));}
  w.machine.advance(now,&mut w.wire);let reply=w.machine.pending_reply();
  if reply.is_some(){w.machine.reply_accepted();}reply
 };
 if let Some((_,word))=reply{p.reply(word,now);}
}
fn finish(p:&mut probe::Client,from:u64){for now in from..from+10000{turn(p,now);if !p.busy(){return;}}panic!("client did not finish");}
#[test]fn idle_client_never_queries_without_explicit_user_intent(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();
 assert_eq!(p.view(),probe::View::Idle);assert!(!p.busy());for now in 0..20{turn(&mut p,now);}
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
 p.start();p.cancel();turn(&mut p,21);assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
}
#[test]fn actual_client_abi_codec_and_job_answer_and_release_with_each_enqueue_full_once(){
 let _serial=SERIAL.lock().unwrap();reset(true);let mut p=probe::Client::new();p.start();finish(&mut p,1);
 assert_eq!(p.view(),probe::View::Answer([10,0,2,2]));let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();
 assert_eq!(w.wire.begins,vec![1]);assert_eq!(w.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![1,2,3,5]);
 assert_eq!(w.calls,w.accepted.len()*2);assert_eq!(w.machine.owner(),None);
}
#[test]fn wrong_tag_or_sequence_does_not_consume_accepted_correlation(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start();p.advance(1);
 p.reply(network_job::reply(1,1),2);p.reply(network_dns::reply(2,1),2);
 p.advance(2001);assert_eq!(p.view(),probe::View::Uncertain);assert!(p.busy());
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);
 finish(&mut p,2002);assert_eq!(p.view(),probe::View::Cancelled);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner(),None);
}
#[test]fn waiting_status_polls_at_most_once_per_twenty_five_ticks(){
 let _serial=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().wire.answer=false;
 let mut p=probe::Client::new();p.start();turn(&mut p,1);turn(&mut p,2);
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.len(),2);
 for now in 3..27{turn(&mut p,now);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.len(),2);
 turn(&mut p,27);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.len(),3);
 p.cancel();finish(&mut p,28);assert_eq!(p.view(),probe::View::Cancelled);
}
#[test]fn failed_begin_still_acknowledges_ticket_and_releases_before_retry(){
 let _serial=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().wire.error=Some(network_dns::OFFLINE);
 let mut p=probe::Client::new();p.start();finish(&mut p,1);assert_eq!(p.view(),probe::View::Unavailable);
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner(),None);
 WORLD.lock().unwrap().as_mut().unwrap().wire.error=None;p.start();finish(&mut p,100);
 assert_eq!(p.view(),probe::View::Answer([10,0,2,2]));assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().wire.begins,vec![1,2]);
}
#[test]fn late_accepted_begin_is_retained_until_cleanup_before_queued_retry(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start();p.advance(1);p.advance(2001);p.start();
 assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);
 finish(&mut p,2002);assert_eq!(p.view(),probe::View::Answer([10,0,2,2]));
 let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.wire.begins,vec![1,2]);
 assert_eq!(w.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![1,4,5,1,2,3,5]);assert_eq!(w.machine.owner(),None);
}
#[test]fn late_accepted_release_must_arrive_before_retry_and_cancel_clears_retry_intent(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start();
 turn(&mut p,1);turn(&mut p,2);turn(&mut p,3);p.advance(4);p.advance(2004);p.start();p.cancel();
 assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,4);
 finish(&mut p,2005);assert!(!p.busy());
 let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.wire.begins,vec![1]);assert_eq!(w.machine.owner(),None);
}
#[test]fn preeffect_full_has_bounded_retry_without_allocating_a_job(){
 let _serial=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().full_always=true;
 let mut p=probe::Client::new();p.start();p.advance(1);p.advance(100);p.advance(2001);
 assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);
 assert!(WORLD.lock().unwrap().as_ref().unwrap().accepted.is_empty());
 WORLD.lock().unwrap().as_mut().unwrap().full_always=false;p.start();finish(&mut p,2002);
 assert_eq!(p.view(),probe::View::Answer([10,0,2,2]));
}
#[test]fn status_replacement_full_retains_chunk_coalesces_latest_and_formats_decimal(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut s=probe::Status::new(128);
 WORLD.lock().unwrap().as_mut().unwrap().display_full=true;s.advance();assert!(s.pending());
 assert!(WORLD.lock().unwrap().as_ref().unwrap().display.is_empty());
 WORLD.lock().unwrap().as_mut().unwrap().display_full=false;s.advance();s.set(probe::View::Uncertain);s.set(probe::View::Answer([0,9,100,255]));
 for _ in 0..30{s.advance();}assert!(!s.pending());let guard=WORLD.lock().unwrap();let chunks=&guard.as_ref().unwrap().display;
 assert_eq!(chunks.len(),16);let mut bytes=[0;64];for(offset,data)in chunks{bytes[offset-128..offset-120].copy_from_slice(data);}
 assert!(bytes.starts_with(b"\n\nControlled DNS: 0.9.100.255\nD query  X cancel"));assert!(bytes.ends_with(b" "));
 drop(guard);let mut s=probe::Status::new(usize::MAX);s.advance();assert!(!s.pending());
}
#[test]fn accepted_release_timeout_never_starts_retry_until_positive_late_cleanup(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start();
 turn(&mut p,1);turn(&mut p,2);turn(&mut p,3);p.advance(4);p.advance(2004);p.start();p.advance(2005);
 assert_eq!(p.view(),probe::View::Uncertain);
 {let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.calls,4);assert_eq!(w.wire.begins,vec![1]);assert!(w.machine.owner().is_some());}
 turn(&mut p,2006);assert!(p.busy());assert_eq!(p.view(),probe::View::Pending);
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().wire.begins,vec![1]);
 finish(&mut p,2007);assert_eq!(p.view(),probe::View::Answer([10,0,2,2]));
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().wire.begins,vec![1,2]);
}
#[test]fn cancel_while_result_receipt_is_pending_finishes_cancelled_after_cleanup(){
 let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start();
 turn(&mut p,1);turn(&mut p,2);p.advance(3);p.cancel();finish(&mut p,4);
 assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);
 let g=WORLD.lock().unwrap();assert_eq!(g.as_ref().unwrap().machine.owner(),None);
}
