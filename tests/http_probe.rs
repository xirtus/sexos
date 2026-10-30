// SPDX-License-Identifier: GPL-3.0-only
//! Actual HTTP Client, network-job ABI/machine and complete-message HTTP codec.
extern crate self as sex_pdx;
pub use sex_net_config as network_config;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../apps/kaleidoscope/src/http_wire.rs"]mod http_wire;
#[path="../apps/kaleidoscope/src/http_probe.rs"]mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
#[macro_export]macro_rules! serial_record {($($t:tt)*)=>{let _=std::format!($($t)*);};}
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
#[derive(Clone,Copy)]enum Mode { Valid, Malformed, Disconnect, Waiting, Raw }
struct Peer {link:jobs::Link,request:Vec<u8>,response:Vec<u8>,read:usize,body:Vec<u8>,mode:Mode,begins:Vec<u32>,path:http_wire::Path}
impl jobs::Wire for Peer {
 fn begin(&mut self,ticket:u32)->Result<(),u32>{self.begins.push(ticket);self.request.clear();self.response.clear();self.read=0;self.link=jobs::Link::Open;Ok(())}
 fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{let n=out.len().min(11).min(self.response.len()-self.read);out[..n].copy_from_slice(&self.response[self.read..self.read+n]);self.read+=n;Ok(n)}
 fn send(&mut self,data:&[u8])->Result<usize,u32>{let n=data.len().min(7);self.request.extend_from_slice(&data[..n]);Ok(n)}
 fn queued(&self)->usize{0}fn link(&self)->jobs::Link{self.link}
 fn finish_send(&mut self){
  let mut expected=[0;http_wire::MAX_REQUEST];let length=http_wire::request_path(&mut expected,&self.path);
  assert_eq!(self.request,&expected[..length]);
  match self.mode {
   Mode::Valid=>{self.response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",self.body.len()).into_bytes();self.response.extend_from_slice(&self.body);self.link=jobs::Link::PeerFin;},
   Mode::Malformed=>{self.response=b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain\r\n\r\nbadEXTRA".to_vec();self.link=jobs::Link::PeerFin;},
   Mode::Disconnect=>self.link=jobs::Link::Closed,Mode::Waiting=>{},
   Mode::Raw=>{self.response=self.body.clone();self.link=jobs::Link::PeerFin;},
  }
 }
 fn abort(&mut self){self.link=jobs::Link::Closed;self.request.clear();self.response.clear();self.read=0;}
}
struct World {machine:jobs::Machine,wire:Peer,request:Option<network_job::Request>,accepted:Vec<network_job::Request>,
 calls:usize,full_each:bool,last_full:u32,full_always:bool,full_kind:Option<u8>,reject_release:bool}
fn reset(full:bool,mode:Mode,body:&[u8]){*WORLD.lock().unwrap()=Some(World {
 machine:jobs::Machine::new(),wire:Peer {link:jobs::Link::Closed,request:Vec::new(),response:Vec::new(),read:0,body:body.to_vec(),mode,begins:Vec::new(),path:http_wire::Path::initial()},
 request:None,accepted:Vec::new(),calls:0,full_each:full,last_full:0,full_always:false,full_kind:None,reject_release:false,
});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
 let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();assert_eq!(slot,SLOT_NET);assert_eq!(op,network_job::OP);
 let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());w.calls+=1;
 if w.full_always||w.full_kind==Some(r.kind)||(w.full_each&&w.last_full!=r.seq){w.last_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
 assert!(w.request.is_none());assert!(w.accepted.last().is_none_or(|prior|r.seq>prior.seq),"accepted RPC replayed or sequence regressed");w.accepted.push(r);w.request=Some(r);(0,0)
}
fn pump(p:&mut probe::Client,now:u64){
 let reply={let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();let owner=jobs::Identity {pd:15,task:15};
  let rejected=if let Some(r)=w.request.take(){
   if r.kind==network_job::RELEASE&&w.reject_release{Some(network_job::error(r.seq,network_job::IO))}
   else{
    let admission=if r.a==1 {Ok(jobs::Resolution{owner,generation:1,address:[10,0,2,2],observed_tick:0,expires_tick:60000})}else{Err(network_job::DENIED)};
    assert!(w.machine.handle_resolved(owner,r,now,false,admission,&mut w.wire));None
   }
  }else{None};
  w.machine.advance(now,&mut w.wire);
  rejected.or_else(||{let reply=w.machine.pending_reply();if reply.is_some(){w.machine.reply_accepted();}reply.map(|(_,word)|word)})
 };if let Some(word)=reply{p.reply(word,now);}
}
fn turn(p:&mut probe::Client,now:u64){p.advance(now);pump(p,now);}
fn finish(p:&mut probe::Client,from:u64){for now in from..from+15000{turn(p,now);if !p.busy(){return;}}panic!("client did not finish");}
fn pending(p:&mut probe::Client,kind:u8,from:u64)->u64 {
 for now in from..from+10000 {p.advance(now);if WORLD.lock().unwrap().as_ref().unwrap().request.is_some_and(|r|r.kind==kind){return now;}pump(p,now);}
 panic!("no pending kind={kind}");
}
fn no_preview(p:&probe::Client){assert_eq!(p.preview(),&[b' ';64]);assert_eq!(p.body_length(),0);assert!(p.document().is_empty());}
#[test]fn explicit_start_uses_monotonic_echo_handoff_and_partial_full_actual_http_job(){
 let _serial=SERIAL.lock().unwrap();reset(true,Mode::Valid,b"controlled response\n");
 {let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();let owner=jobs::Identity {pd:15,task:15};
  assert!(w.machine.handle(owner,network_job::Request {seq:50,kind:network_job::BEGIN,offset:0,length:1,ticket:0,a:0},0,&mut w.wire));w.machine.reply_accepted();
  assert!(w.machine.handle(owner,network_job::Request {seq:51,kind:network_job::RELEASE,offset:0,length:0,ticket:1,a:0},0,&mut w.wire));w.machine.reply_accepted();
 }
 let mut p=probe::Client::new();assert_eq!(p.view(),probe::View::Idle);turn(&mut p,1);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
 assert!(!p.start(0));assert!(p.start(52));assert!(!p.start(100));finish(&mut p,2);
 assert_eq!(p.view(),probe::View::Ready);assert_eq!(p.body_length(),20);assert!(p.preview().starts_with(b"controlled response "));assert!(p.raw_scrubbed());
 let next=p.next_sequence();assert!(next>52);assert!(!p.start(next-1));
 let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.wire.begins,vec![2]);assert!(w.calls>w.accepted.len());assert!(w.accepted.iter().all(|r|r.seq>=52));assert!(w.machine.owner().is_none());
}
#[test]fn validated_body_is_not_ready_or_public_until_positive_release_receipt(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"validated body");let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::RELEASE,1);assert_eq!(p.view(),probe::View::Pending);assert!(p.busy());no_preview(&p);
 pump(&mut p,now);assert_eq!(p.view(),probe::View::Ready);assert!(p.preview().starts_with(b"validated body"));assert!(p.raw_scrubbed());
}
#[test]fn malformed_http_or_disconnect_cleans_up_without_publishing_old_preview(){
 let _serial=SERIAL.lock().unwrap();for mode in[Mode::Malformed,Mode::Disconnect]{
  reset(false,Mode::Valid,b"old body");let mut p=probe::Client::new();assert!(p.start(1));finish(&mut p,1);assert_eq!(p.view(),probe::View::Ready);
  WORLD.lock().unwrap().as_mut().unwrap().wire.mode=mode;assert!(p.start(p.next_sequence()));no_preview(&p);finish(&mut p,1000);
  assert_eq!(p.view(),probe::View::Unavailable);no_preview(&p);assert!(p.raw_scrubbed());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
 }
}
#[test]fn wrong_tag_and_sequence_do_not_consume_accepted_begin_then_late_cancel_cleans(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");let mut p=probe::Client::new();assert!(p.start(101));p.advance(1);
 p.reply((0x79u64<<56)|(101u64<<32)|1,2);p.reply(network_job::reply(102,1),2);p.advance(2001);
 assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);assert!(!p.start(500));
 p.cancel();finish(&mut p,2002);assert_eq!(p.view(),probe::View::Cancelled);assert!(p.raw_scrubbed());no_preview(&p);
 assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![1,6,7]);
}
#[test]fn cancelling_unadmitted_begin_or_full_write_never_replays_abandoned_mutation(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");let mut p=probe::Client::new();assert!(p.start(1));p.cancel();turn(&mut p,1);
 assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
 assert!(p.start(p.next_sequence()));turn(&mut p,2);WORLD.lock().unwrap().as_mut().unwrap().full_kind=Some(network_job::WRITE);p.advance(3);p.cancel();
 WORLD.lock().unwrap().as_mut().unwrap().full_kind=None;finish(&mut p,4);
 assert_eq!(p.view(),probe::View::Cancelled);assert!(p.raw_scrubbed());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![1,6,7]);
}
#[test]fn late_accepted_read_is_discarded_after_cancel_and_release_is_still_required(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"late body");let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::READ,1);p.advance(now+2000);p.cancel();assert_eq!(p.view(),probe::View::Uncertain);
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;p.advance(now+2001);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 pump(&mut p,now+2002);finish(&mut p,now+2003);assert_eq!(p.view(),probe::View::Cancelled);no_preview(&p);assert!(p.raw_scrubbed());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
}
#[test]fn late_accepted_release_never_restarts_and_last_cancel_intent_wins(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::RELEASE,1);p.advance(now+2000);assert_eq!(p.view(),probe::View::Uncertain);assert!(!p.start(p.next_sequence()+10));p.cancel();
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;p.advance(now+2001);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 pump(&mut p,now+2002);assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);no_preview(&p);assert!(p.raw_scrubbed());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().wire.begins.len(),1);
}
#[test]fn failed_release_preserves_owned_custody_until_explicit_cleanup_succeeds(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");WORLD.lock().unwrap().as_mut().unwrap().reject_release=true;
 let mut p=probe::Client::new();assert!(p.start(1));let now=pending(&mut p,network_job::RELEASE,1);pump(&mut p,now);
 assert!(p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert!(!p.start(p.next_sequence()));no_preview(&p);
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;for at in now+1..now+20{p.advance(at);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 WORLD.lock().unwrap().as_mut().unwrap().reject_release=false;p.cancel();finish(&mut p,now+21);assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);
}
#[test]fn preview_normalizes_ascii_pads_and_marks_actual_truncation(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b" \nA\t\tB\0\xFF  ");let mut p=probe::Client::new();assert!(p.start(1));finish(&mut p,1);
 assert!(p.preview().starts_with(b"A B ? "));assert_eq!(p.body_length(),10);assert!(p.preview().iter().all(|b|(32..=126).contains(b)));
 WORLD.lock().unwrap().as_mut().unwrap().wire.body=vec![b'X';65];assert!(p.start(p.next_sequence()));finish(&mut p,1000);assert_eq!(&p.preview()[61..],b"...");assert_eq!(p.body_length(),65);
}
#[test]fn finite_sequence_budget_always_reserves_cancel_and_release_without_wrap(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Waiting,b"body");let mut p=probe::Client::new();
 assert!(!p.start(network_job::MAX_SEQ-7));assert!(p.start(network_job::MAX_SEQ-8));finish(&mut p,1);
 assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.next_sequence(),network_job::MAX_SEQ+1);
 assert!(p.raw_scrubbed());assert!(!p.start(network_job::MAX_SEQ-8));assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
}
#[test]fn preeffect_full_begin_times_out_without_holding_or_automatically_retrying(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");WORLD.lock().unwrap().as_mut().unwrap().full_always=true;
 let mut p=probe::Client::new();assert!(p.start(1));p.advance(1);p.advance(2001);assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);assert!(p.raw_scrubbed());
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;p.advance(2002);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);no_preview(&p);
}
#[test]fn invalid_accepted_begin_ticket_stays_uncertain_and_cannot_admit_new_work(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"body");let mut p=probe::Client::new();assert!(p.start(1));p.advance(1);
 p.reply(network_job::reply(1,0),2);assert!(p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert!(!p.start(100));p.cancel();
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;p.advance(3);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 assert!(p.busy());assert!(p.raw_scrubbed());no_preview(&p);
}
#[test]fn full_maximum_document_is_hidden_until_exact_positive_release_and_raw_is_scrubbed(){
 let _serial=SERIAL.lock().unwrap();let body:Vec<u8>=(0..http_wire::MAX_BODY).map(|n|b'A'+(n%26)as u8).collect();
 reset(false,Mode::Valid,&body);let mut p=probe::Client::new();assert!(p.document_scrubbed());assert!(p.start_resolved(1,1));
 let now=pending(&mut p,network_job::RELEASE,1);no_preview(&p);assert!(!p.document_scrubbed());assert!(!p.raw_scrubbed());
 let request=WORLD.lock().unwrap().as_ref().unwrap().request.unwrap();
 p.reply(network_job::reply(request.seq+1,0),now);p.reply((0x79u64<<56)|(request.seq as u64)<<32,now);
 no_preview(&p);assert!(p.busy());pump(&mut p,now);
 assert_eq!(p.view(),probe::View::Ready);assert_eq!(p.body_length(),http_wire::MAX_BODY);assert_eq!(p.document(),body);
 assert_eq!(&p.document()[http_wire::MAX_BODY-26..],&body[http_wire::MAX_BODY-26..]);assert_eq!(&p.preview()[61..],b"...");assert!(p.raw_scrubbed());
 p.cancel();no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
}
#[test]fn document_retains_every_received_byte_without_presentation_rewriting(){
 let _serial=SERIAL.lock().unwrap();let body:Vec<u8>=(0..http_wire::MAX_BODY).map(|n|(n%256)as u8).collect();

 reset(true,Mode::Valid,&body);let mut p=probe::Client::new();assert!(p.start_resolved(1,1));finish(&mut p,1);
 assert_eq!(p.document(),body);assert_eq!(p.document().len(),body.len());assert_eq!(p.document()[10],10);
 assert_eq!(p.document()[9],9);assert_eq!(p.document()[13],13);assert_eq!(p.document()[127],127);assert_eq!(p.document()[255],255);
 assert_eq!(&p.document()[2040..],&body[2040..]);assert!(p.raw_scrubbed());
}
#[test]fn shorter_replacement_has_exact_extent_and_reset_scrubs_the_entire_old_document(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,&vec![b'S';http_wire::MAX_BODY]);let mut p=probe::Client::new();assert!(p.start_resolved(1,1));finish(&mut p,1);
 assert_eq!(p.document().len(),http_wire::MAX_BODY);WORLD.lock().unwrap().as_mut().unwrap().wire.body=b"tail\n  A\tB\xFF".to_vec();
 assert!(p.start_resolved(p.next_sequence(),1));no_preview(&p);assert!(p.document_scrubbed());finish(&mut p,1000);
 assert_eq!(p.document(),b"tail\n  A\tB\xFF");assert_eq!(p.body_length(),11);assert!(p.raw_scrubbed());
 p.wait_for_resolution();no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());assert_eq!(p.view(),probe::View::Pending);
}
#[test]fn cancel_after_validation_scrubs_pending_document_and_late_release_cannot_publish_it(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,&vec![b'L';257]);let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::RELEASE,1);let request=WORLD.lock().unwrap().as_ref().unwrap().request.unwrap();
 assert!(!p.document_scrubbed());p.cancel();no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
 pump(&mut p,now);assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());assert!(p.document_scrubbed());
 p.reply(network_job::reply(request.seq,0),now+1);no_preview(&p);assert!(p.document_scrubbed());
 WORLD.lock().unwrap().as_mut().unwrap().wire.body=b"fresh\nbody".to_vec();assert!(p.start(p.next_sequence()));
 p.reply(network_job::reply(request.seq,0),now+2);no_preview(&p);finish(&mut p,now+3);
 assert_eq!(p.document(),b"fresh\nbody");assert!(p.raw_scrubbed());
}
#[test]fn accepted_release_timeout_hides_and_scrubs_validated_document_without_replay(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,&vec![b'T';1025]);let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::RELEASE,1);assert!(!p.document_scrubbed());p.advance(now+2000);
 assert_eq!(p.view(),probe::View::Uncertain);no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;p.advance(now+2001);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 pump(&mut p,now+2002);assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());no_preview(&p);assert!(p.document_scrubbed());
}
#[test]fn rejected_release_clears_validated_document_while_preserving_owned_cleanup(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,&vec![b'R';129]);let mut p=probe::Client::new();assert!(p.start(1));
 let now=pending(&mut p,network_job::RELEASE,1);assert!(!p.document_scrubbed());WORLD.lock().unwrap().as_mut().unwrap().reject_release=true;pump(&mut p,now);
 assert_eq!(p.view(),probe::View::Uncertain);assert!(p.busy());no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
 WORLD.lock().unwrap().as_mut().unwrap().reject_release=false;p.cancel();finish(&mut p,now+1);assert_eq!(p.view(),probe::View::Cancelled);assert!(p.document_scrubbed());
}
#[test]fn unavailable_response_or_rejected_new_intent_never_retains_prior_document(){
 let _serial=SERIAL.lock().unwrap();for mode in [Mode::Malformed,Mode::Disconnect]{
  reset(false,Mode::Valid,&vec![b'Q';2048]);let mut p=probe::Client::new();assert!(p.start(1));finish(&mut p,1);assert_eq!(p.document().len(),2048);
  WORLD.lock().unwrap().as_mut().unwrap().wire.mode=mode;assert!(p.start(p.next_sequence()));assert!(p.document_scrubbed());finish(&mut p,1000);
  assert_eq!(p.view(),probe::View::Unavailable);no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
 }
 reset(false,Mode::Valid,b"old document");let mut p=probe::Client::new();assert!(p.start(1));finish(&mut p,1);assert!(!p.start(p.next_sequence()-1));
 no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
}
#[test]fn invalid_resolution_and_empty_success_have_no_document_bytes(){
 let _serial=SERIAL.lock().unwrap();reset(false,Mode::Valid,b"old document");let mut p=probe::Client::new();assert!(p.start(1));finish(&mut p,1);
 assert!(!p.start_resolved(p.next_sequence(),0));assert_eq!(p.view(),probe::View::Unavailable);no_preview(&p);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
 WORLD.lock().unwrap().as_mut().unwrap().wire.body.clear();assert!(p.start(p.next_sequence()));finish(&mut p,1000);
 assert_eq!(p.view(),probe::View::Ready);assert_eq!(p.body_length(),0);assert!(p.document().is_empty());assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
}

fn redirect_response(path:&str,body:&[u8])->Vec<u8>{let mut raw=format!("HTTP/1.1 302 Found\r\nLocation: {path}\r\nContent-Length: {}\r\nContent-Type: text/html; charset=UTF-8\r\n\r\n",body.len()).into_bytes();raw.extend_from_slice(body);raw}
#[test]fn redirect_is_hidden_until_positive_release_and_fresh_path_request_is_explicit(){
 let _serial=SERIAL.lock().unwrap();reset(true,Mode::Raw,&redirect_response("/guide",b"not a document"));let mut p=probe::Client::new();assert!(p.start_resolved_path(1,1,http_wire::Path::initial()));
 let now=pending(&mut p,network_job::RELEASE,1);assert!(p.busy());assert!(p.take_redirect().is_none());no_preview(&p);assert!(p.document_scrubbed());
 pump(&mut p,now);assert!(!p.busy());assert_eq!(p.view(),probe::View::Pending);no_preview(&p);assert!(p.document_path().is_empty());
 let path=p.take_redirect().unwrap();assert_eq!(path.as_bytes(),b"/guide");assert!(p.take_redirect().is_none());
 let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;for at in now+1..now+5{p.advance(at);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
 {let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();assert!(w.machine.owner().is_none());assert_eq!(w.wire.begins.len(),1);w.wire.mode=Mode::Valid;w.wire.body=b"fresh final document".to_vec();w.wire.path=path;}
 assert!(p.start_resolved_path(p.next_sequence(),1,path));finish(&mut p,now+5);assert_eq!(p.view(),probe::View::Ready);assert_eq!(p.document(),b"fresh final document");assert_eq!(p.document_path(),b"/guide");assert!(p.take_redirect().is_none());
}
#[test]fn rejected_or_late_redirect_release_never_authorizes_followup(){
 let _serial=SERIAL.lock().unwrap();for late in [false,true]{
  reset(false,Mode::Raw,&redirect_response("/guide",b"discard me"));let mut p=probe::Client::new();assert!(p.start_resolved_path(1,1,http_wire::Path::initial()));let now=pending(&mut p,network_job::RELEASE,1);
  if late{p.advance(now+2000);}else{WORLD.lock().unwrap().as_mut().unwrap().reject_release=true;pump(&mut p,now);}
  assert!(p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert!(p.take_redirect().is_none());no_preview(&p);assert!(p.document_scrubbed());
  WORLD.lock().unwrap().as_mut().unwrap().reject_release=false;p.cancel();finish(&mut p,now+2001);assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);assert!(p.take_redirect().is_none());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().wire.begins.len(),1);
 }
}
#[test]fn each_accepted_redirect_phase_can_be_cancelled_without_continuation_or_body(){
 let _serial=SERIAL.lock().unwrap();for kind in [network_job::BEGIN_STREAM_RESOLVED,network_job::WRITE,network_job::START,network_job::STATUS,network_job::READ,network_job::RELEASE]{
  reset(false,Mode::Raw,&redirect_response("/guide",b"private discarded body"));let mut p=probe::Client::new();assert!(p.start_resolved_path(1,1,http_wire::Path::initial()));let now=pending(&mut p,kind,1);p.cancel();finish(&mut p,now+1);
  assert!(!p.busy(),"kind={kind}");assert_eq!(p.view(),probe::View::Cancelled);assert!(p.take_redirect().is_none());no_preview(&p);assert!(p.document_scrubbed());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
 }
}
#[test]fn redirect_framing_and_scope_failures_release_without_continuation(){
 let _serial=SERIAL.lock().unwrap();let mut short=redirect_response("/guide",b"abc");short.pop();let mut trailing=redirect_response("/guide",b"abc");trailing.push(b'x');
 let responses=[short,trailing,redirect_response("http://elsewhere.test/guide",b""),redirect_response("/guide",&vec![b'x';http_wire::MAX_REDIRECT_BODY+1])];
 for raw in responses{reset(false,Mode::Raw,&raw);let mut p=probe::Client::new();assert!(p.start_resolved_path(1,1,http_wire::Path::initial()));finish(&mut p,1);assert_eq!(p.view(),probe::View::Unavailable);assert!(p.take_redirect().is_none());no_preview(&p);assert!(p.document_scrubbed());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());}
}

#[test]fn utf8_charset_and_split_scalars_retain_exact_bytes_only_after_positive_release(){
 let _serial=SERIAL.lock().unwrap();let body="café\t中文\n😀 raw".as_bytes();
 let mut response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",body.len()).into_bytes();response.extend_from_slice(body);
 reset(true,Mode::Raw,&response);let mut p=probe::Client::new();assert!(p.start_resolved(1,1));let now=pending(&mut p,network_job::RELEASE,1);
 assert!(p.document().is_empty());assert_eq!(p.view(),probe::View::Pending);pump(&mut p,now);assert_eq!(p.document(),body);assert_eq!(p.view(),probe::View::Ready);
 p.cancel();assert!(p.document_scrubbed());
}
