// SPDX-License-Identifier: GPL-3.0-only
//! Actual Browser client + register ABI + job machine; explicit transport model.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../apps/kaleidoscope/src/network_probe.rs"]mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const SLOT_DISPLAY:u64=5;pub const ERR_SERVICE_NOT_READY:u64=9;
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{let _=std::format!($($t)*);};}
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
struct Peer{link:jobs::Link,bytes:Vec<u8>,at:usize,aborts:usize}
impl jobs::Wire for Peer{
 fn begin(&mut self,_:u32)->Result<(),u32>{self.link=jobs::Link::Open;self.bytes.clear();self.at=0;Ok(())}
 fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{let n=out.len().min(11).min(self.bytes.len()-self.at);out[..n].copy_from_slice(&self.bytes[self.at..self.at+n]);self.at+=n;Ok(n)}
 fn send(&mut self,data:&[u8])->Result<usize,u32>{let n=data.len().min(7);self.bytes.extend_from_slice(&data[..n]);Ok(n)}
 fn queued(&self)->usize{0}fn link(&self)->jobs::Link{self.link}
 fn finish_send(&mut self){self.link=jobs::Link::PeerFin;}
 fn abort(&mut self){self.link=jobs::Link::Closed;self.bytes.clear();self.at=0;self.aborts+=1;}
}
struct World{machine:jobs::Machine,wire:Peer,request:Option<network_job::Request>,accepted:Vec<u32>,calls:usize,
 full_each:bool,last_full:u32,display:Vec<(usize,[u8;8])>,display_full:bool}
fn reset(full:bool){*WORLD.lock().unwrap()=Some(World{machine:jobs::Machine::new(),wire:Peer{link:jobs::Link::Closed,bytes:Vec::new(),at:0,aborts:0},request:None,accepted:Vec::new(),calls:0,full_each:full,last_full:0,display:Vec::new(),display_full:false});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();
 if slot==SLOT_DISPLAY{assert_eq!(op,0xFB);assert_eq!(a,300);assert_eq!((c>>8)&255,8);if w.display_full{return(ERR_SERVICE_NOT_READY,0);}w.display.push(((c&255)as usize,b.to_le_bytes()));return(0,0);}
 assert_eq!(slot,SLOT_NET);assert_eq!(op,network_job::OP);let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());w.calls+=1;
 if w.full_each&&w.last_full!=r.seq{w.last_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
 assert!(w.request.is_none());assert!(!w.accepted.contains(&r.seq),"accepted mutation replayed");w.accepted.push(r.seq);w.request=Some(r);(0,0)
}
fn turn(p:&mut probe::Probe,now:u64){p.advance(now);let reply={let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();let owner=jobs::Identity{pd:15,task:15};if let Some(r)=w.request.take(){assert!(w.machine.handle(owner,r,now,&mut w.wire));}w.machine.advance(now,&mut w.wire);let reply=w.machine.pending_reply();if reply.is_some(){w.machine.reply_accepted();}reply};if let Some((_,word))=reply{p.reply(word,now);}}
#[test]fn actual_client_job_abi_complete_with_partial_io_and_every_enqueue_full_once(){let _serial=SERIAL.lock().unwrap();reset(true);let mut p=probe::Probe::new();for now in 1..5000{turn(&mut p,now);if !p.busy(){break;}}assert!(!p.busy());assert_eq!(p.view(),probe::View::Verified);let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none());assert!(w.calls>w.accepted.len());assert!(w.accepted.len()>350);}
#[test]fn cancel_and_release_then_retry_uses_fresh_ticket_without_reusing_sequence(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();for now in 1..100{turn(&mut p,now);if now==20{p.cancel();}if !p.busy(){break;}}assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());p.retry();for now in 100..5000{turn(&mut p,now);if !p.busy(){break;}}assert_eq!(p.view(),probe::View::Verified);let g=WORLD.lock().unwrap();assert_eq!(g.as_ref().unwrap().machine.snapshot().ticket,0);}
#[test]fn accepted_request_has_no_automatic_timeout_replay_or_wrong_sequence_reply(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();p.advance(1);p.reply(network_job::reply(2,1),2);for now in 2..2002{p.advance(now);}assert!(p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);}
#[test]fn status_replacement_retains_full_chunk_and_finishes_latest_padded_text(){let _serial=SERIAL.lock().unwrap();reset(false);let mut s=probe::Status::new(140);WORLD.lock().unwrap().as_mut().unwrap().display_full=true;s.advance();assert!(s.pending());assert!(WORLD.lock().unwrap().as_ref().unwrap().display.is_empty());WORLD.lock().unwrap().as_mut().unwrap().display_full=false;s.advance();s.set(probe::View::Cancelled);for _ in 0..20{s.advance();}assert!(!s.pending());let g=WORLD.lock().unwrap();let v=&g.as_ref().unwrap().display;let mut text=[0u8;64];for(offset,bytes)in v{text[offset-140..offset-132].copy_from_slice(bytes);}assert!(String::from_utf8_lossy(&text).contains("cancelled"));assert!(text.ends_with(b" "));}

#[test]fn late_accepted_begin_retains_correlation_and_releases_before_requested_retry(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();p.advance(1);p.advance(2002);assert_eq!(p.view(),probe::View::Uncertain);p.retry();assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);for now in 2003..9000{turn(&mut p,now);if !p.busy(){break;}}assert_eq!(p.view(),probe::View::Verified);assert!(!p.busy());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());}
#[test]fn explicit_cancel_overrides_queued_retry_before_late_begin_receipt(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();p.advance(1);p.retry();p.cancel();for now in 2..100{turn(&mut p,now);if !p.busy(){break;}}assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none());assert_eq!(w.accepted.len(),3);}
#[test]fn route_sequence_handoff_requires_positive_release_and_cannot_regress(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();assert!(!p.restart_at(100));p.cancel();for now in 1..20{turn(&mut p,now);if !p.busy(){break;}}assert!(!p.busy());let seq=p.next_sequence();assert!(!p.restart_at(seq-1));assert!(p.restart_at(seq));for now in 30..5000{turn(&mut p,now);if !p.busy(){break;}}assert_eq!(p.view(),probe::View::Verified);assert!(p.next_sequence()>seq);}
#[test]fn foreign_reply_tag_does_not_consume_accepted_echo_request(){let _serial=SERIAL.lock().unwrap();reset(false);let mut p=probe::Probe::new();p.advance(1);p.reply((0x79u64<<56)|(1<<32)|1,2);assert!(p.busy());turn(&mut p,3);p.cancel();for now in 4..30{turn(&mut p,now);if !p.busy(){break;}}assert_eq!(p.view(),probe::View::Cancelled);assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());}
