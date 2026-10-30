// SPDX-License-Identifier: GPL-3.0-only
//! Actual DNS→released provenance→HTTP clients, both register ABIs, private job
//! machines and codecs. Explicit mock PDX/wires supply inputs; no BOOT liveness,
//! real NIC, DNS authenticity or endpoint authority is established by this test.
extern crate self as sex_pdx;
pub use sex_net_config as network_config;
extern crate self as sex_net;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/network_dns.rs"]pub mod network_dns;
#[path="../crates/sex-net/src/dns_wire.rs"]pub mod dns_wire;
#[path="../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../servers/sexnet/src/dns_jobs.rs"]mod dns_jobs;
#[path="../apps/kaleidoscope/src/dns_probe.rs"]mod dns_probe;
#[path="../apps/kaleidoscope/src/http_wire.rs"]mod http_wire;
#[path="../apps/kaleidoscope/src/http_probe.rs"]mod http_probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const SLOT_DISPLAY:u64=5;pub const ERR_SERVICE_NOT_READY:u64=9;
#[macro_export]macro_rules! serial_record {($($t:tt)*)=>{let _=std::format!($($t)*);};}
const A:jobs::Identity=jobs::Identity {pd:15,task:15};
const B:jobs::Identity=jobs::Identity {pd:15,task:99};
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Envelope {
 Dns(jobs::Identity,network_dns::Request),Tcp(jobs::Identity,network_job::Request),
}
struct DnsPeer {address:[u8;4],ttl:u32,response:Option<Vec<u8>>,begins:Vec<u32>}
impl dns_jobs::Wire for DnsPeer {
 fn begin(&mut self,ticket:u32)->Result<(),u32>{
  self.begins.push(ticket);
  let mut bytes=vec![0;34];bytes[..2].copy_from_slice(&(ticket as u16).to_be_bytes());bytes[2..8].copy_from_slice(&[0x81,0x80,0,1,0,1]);
  bytes[12..].copy_from_slice(b"\x05probe\x05sexos\x04test\x00\x00\x01\x00\x01");
  bytes.extend_from_slice(&[0xC0,12,0,1,0,1]);bytes.extend_from_slice(&self.ttl.to_be_bytes());bytes.extend_from_slice(&[0,4]);bytes.extend_from_slice(&self.address);
  self.response=Some(bytes);Ok(())
 }
 fn receive(&mut self,out:&mut[u8;512])->Result<Option<usize>,u32>{
  if let Some(bytes)=self.response.take(){out[..bytes.len()].copy_from_slice(&bytes);Ok(Some(bytes.len()))}else{Ok(None)}
 }
 fn abort(&mut self){self.response=None;}
}
struct HttpPeer {link:jobs::Link,request:Vec<u8>,response:Vec<u8>,read:usize,begins:Vec<u32>,body:Vec<u8>,allow_fin:bool}
impl jobs::Wire for HttpPeer {
 fn begin(&mut self,ticket:u32)->Result<(),u32>{self.begins.push(ticket);self.link=jobs::Link::Open;self.request.clear();self.response.clear();self.read=0;Ok(())}
 fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{let n=out.len().min(9).min(self.response.len()-self.read);out[..n].copy_from_slice(&self.response[self.read..self.read+n]);self.read+=n;Ok(n)}
 fn send(&mut self,data:&[u8])->Result<usize,u32>{let n=data.len().min(7);self.request.extend_from_slice(&data[..n]);Ok(n)}
 fn queued(&self)->usize{0}fn link(&self)->jobs::Link{self.link}
 fn finish_send(&mut self){
  assert_eq!(self.request,b"GET /welcome HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n");
  self.response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",self.body.len()).into_bytes();self.response.extend_from_slice(&self.body);if self.allow_fin{self.link=jobs::Link::PeerFin;}
 }
 fn abort(&mut self){self.link=jobs::Link::Closed;self.request.clear();self.response.clear();self.read=0;}
}
struct World {
 dns:dns_jobs::Machine,tcp:jobs::Machine,dns_wire:DnsPeer,http_wire:HttpPeer,
 pending:Option<Envelope>,accepted:Vec<Envelope>,caller:jobs::Identity,
 full_once:bool,last_dns_full:u32,last_tcp_full:u32,deliver_dns:bool,deliver_tcp:bool,
}
fn reset(full_once:bool,address:[u8;4],ttl:u32){*WORLD.lock().unwrap()=Some(World {
 dns:dns_jobs::Machine::new(),tcp:jobs::Machine::new(),dns_wire:DnsPeer {address,ttl,response:None,begins:Vec::new()},
 http_wire:HttpPeer {link:jobs::Link::Closed,request:Vec::new(),response:Vec::new(),read:0,begins:Vec::new(),body:b"resolved body".to_vec(),allow_fin:true},
 pending:None,accepted:Vec::new(),caller:A,full_once,last_dns_full:0,last_tcp_full:0,deliver_dns:true,deliver_tcp:true,
});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
 let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();assert_eq!(slot,SLOT_NET);
 let envelope=if op==network_dns::OP {
  let r=network_dns::Request::decode([a,b,c]).unwrap();
  if w.full_once&&w.last_dns_full!=r.seq{w.last_dns_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
  Envelope::Dns(w.caller,r)
 }else{
  assert_eq!(op,network_job::OP);let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());
  if w.full_once&&w.last_tcp_full!=r.seq{w.last_tcp_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
  Envelope::Tcp(w.caller,r)
 };
 let key=|e:Envelope|match e{Envelope::Dns(owner,r)=>(true,owner,r.seq),Envelope::Tcp(owner,r)=>(false,owner,r.seq)};
 assert!(w.pending.is_none());assert!(!w.accepted.iter().any(|prior|key(*prior)==key(envelope)),"accepted client sequence replayed");
 w.accepted.push(envelope);w.pending=Some(envelope);(0,0)
}
/// Copy exact service data and enforce the immutable controlled endpoint policy.
/// This fixture supplies authenticated identities explicitly; the selector is
/// never treated as a capability and cannot select a different IP/port.
fn policy(dns:&dns_jobs::Machine,owner:jobs::Identity,r:network_job::Request,now:u64)->Result<jobs::Resolution,u32>{
 if !matches!(r.kind,network_job::BEGIN_RESOLVED|network_job::BEGIN_STREAM_RESOLVED){return Err(network_job::DENIED);}
 let generation=u32::try_from(r.a).map_err(|_|network_job::DENIED)?;
 let record=dns.resolved(owner,generation,now).ok_or(network_job::DENIED)?;
 if record.address!=[10,0,2,2]{return Err(network_job::DENIED);}
 Ok(jobs::Resolution {owner:record.owner,generation:record.generation,address:record.address,
  observed_tick:record.observed_tick,expires_tick:record.expires_tick})
}
fn route(w:&mut World,envelope:Envelope,now:u64){
 match envelope {
  Envelope::Dns(owner,r)=>assert!(w.dns.handle(owner,r,now,w.tcp.owner().is_some(),&mut w.dns_wire)),
  Envelope::Tcp(owner,r)=>{let admission=policy(&w.dns,owner,r,now);assert!(w.tcp.handle_resolved(owner,r,now,w.dns.owner().is_some(),admission,&mut w.http_wire));},
 }
}
fn pump(dns:&mut dns_probe::Client,http:&mut http_probe::Client,now:u64){
 let(dns_reply,tcp_reply)={let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();
  w.dns.maintain_records(now,|_|true);
  if let Some(envelope)=w.pending.take(){route(w,envelope,now);}
  w.dns.advance(now,&mut w.dns_wire);w.tcp.advance(now,&mut w.http_wire);
  let dns_reply=if w.deliver_dns{let reply=w.dns.pending_reply();if reply.is_some(){w.dns.reply_accepted();}reply.map(|(_,word)|word)}else{None};
  let tcp_reply=if w.deliver_tcp{let reply=w.tcp.pending_reply();if reply.is_some(){w.tcp.reply_accepted();}reply.map(|(_,word)|word)}else{None};
  (dns_reply,tcp_reply)
 };
 if let Some(word)=dns_reply{dns.reply(word,now);}if let Some(word)=tcp_reply{http.reply(word,now);}
}
fn finish_dns(dns:&mut dns_probe::Client,http:&mut http_probe::Client,from:u64)->u64 {
 for now in from..from+5000{dns.advance(now);pump(dns,http,now);if !dns.busy(){return now;}}panic!("DNS client did not complete");
}
fn finish_http(dns:&mut dns_probe::Client,http:&mut http_probe::Client,from:u64)->u64 {
 for now in from..from+10000{http.advance(now);pump(dns,http,now);if !http.busy(){return now;}}panic!("HTTP client did not complete");
}
fn wait_dns_release(dns:&mut dns_probe::Client,http:&mut http_probe::Client,from:u64)->u64 {
 for now in from..from+5000{
  dns.advance(now);
  if WORLD.lock().unwrap().as_ref().unwrap().pending.is_some_and(|e|matches!(e,Envelope::Dns(_,r)if r.kind==network_dns::RELEASE)){return now;}
  pump(dns,http,now);
 }panic!("DNS RELEASE not pending");
}
fn wait_http_release(dns:&mut dns_probe::Client,http:&mut http_probe::Client,from:u64)->u64 {
 for now in from..from+10000{
  http.advance(now);
  if WORLD.lock().unwrap().as_ref().unwrap().pending.is_some_and(|e|matches!(e,Envelope::Tcp(_,r)if r.kind==network_job::RELEASE)){return now;}
  pump(dns,http,now);
 }panic!("HTTP RELEASE not pending");
}
fn request(seq:u32,generation:u32)->network_job::Request{
 let mut out=[0;http_wire::MAX_REQUEST];let length=http_wire::request(&mut out);
 network_job::Request {seq,kind:network_job::BEGIN_STREAM_RESOLVED,offset:0,length:length as u16,ticket:0,a:generation as u64}
}
fn service_call(owner:jobs::Identity,r:network_job::Request,now:u64)->Result<u32,u32>{
 let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.dns.maintain_records(now,|_|true);
 route(w,Envelope::Tcp(owner,r),now);let(subject,word)=w.tcp.pending_reply().unwrap();assert_eq!(subject,owner);w.tcp.reply_accepted();network_job::decode_reply(word,r.seq)
}
fn assert_no_tcp(){let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert!(w.tcp.owner().is_none());assert!(w.http_wire.begins.is_empty());assert!(w.tcp.resolution().is_none());}
#[test]fn complete_chain_waits_for_positive_dns_and_http_release_with_partial_full_inputs(){
 let _serial=SERIAL.lock().unwrap();reset(true,[10,0,2,2],60);
 // A preceding raw Echo reservation used the same caller's0x20C sequence fence.
 {let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();let begin=network_job::Request {seq:400,kind:network_job::BEGIN,offset:0,length:1,ticket:0,a:0};
  assert!(w.tcp.handle(A,begin,0,&mut w.http_wire));w.tcp.reply_accepted();
  assert!(w.tcp.handle(A,network_job::Request {seq:401,kind:network_job::RELEASE,length:0,ticket:1,..begin},0,&mut w.http_wire));w.tcp.reply_accepted();
 }
 let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();http.wait_for_resolution();dns.start();
 let now=wait_dns_release(&mut dns,&mut http,1);assert!(dns.generation().is_none());
 WORLD.lock().unwrap().as_mut().unwrap().deliver_dns=false;pump(&mut dns,&mut http,now);
 for at in now+1..now+10{dns.advance(at);http.advance(at);pump(&mut dns,&mut http,at);assert_eq!(dns.generation(),None);}
 {let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.dns.owned_records(A),1);assert!(w.accepted.iter().all(|e|matches!(e,Envelope::Dns(..))));}
 WORLD.lock().unwrap().as_mut().unwrap().deliver_dns=true;pump(&mut dns,&mut http,now+10);
 let generation=dns.generation().unwrap();assert_eq!(dns.view(),dns_probe::View::Answer([10,0,2,2]));
 assert!(http.start_resolved(402,generation));let release=wait_http_release(&mut dns,&mut http,now+11);
 {let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();let resolved=w.tcp.resolution().unwrap();assert_eq!(resolved.generation,generation);assert_eq!(resolved.owner,A);assert_eq!(resolved.address,[10,0,2,2]);assert_eq!(w.http_wire.begins,vec![2]);}
 WORLD.lock().unwrap().as_mut().unwrap().deliver_tcp=false;pump(&mut dns,&mut http,release);
 assert!(http.busy());assert_eq!(http.view(),http_probe::View::Pending);assert_eq!(http.preview(),&[b' ';64]);assert_eq!(http.body_length(),0);
 WORLD.lock().unwrap().as_mut().unwrap().deliver_tcp=true;pump(&mut dns,&mut http,release+1);
 assert_eq!(http.view(),http_probe::View::Ready);assert_eq!(http.body_length(),13);assert!(http.preview().starts_with(b"resolved body"));assert!(http.raw_scrubbed());
 assert!(WORLD.lock().unwrap().as_ref().unwrap().tcp.owner().is_none());
}
#[test]fn cancelling_dns_after_result_and_before_late_release_prevents_http_handoff(){
 let _serial=SERIAL.lock().unwrap();reset(false,[10,0,2,2],60);let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();http.wait_for_resolution();dns.start();
 let now=wait_dns_release(&mut dns,&mut http,1);assert!(matches!(dns.view(),dns_probe::View::Answer(_)));assert_eq!(dns.generation(),None);
 dns.cancel();pump(&mut dns,&mut http,now);assert_eq!(dns.view(),dns_probe::View::Cancelled);assert_eq!(dns.generation(),None);
 http.cancel();http.advance(now+1);assert_eq!(http.view(),http_probe::View::Cancelled);assert_no_tcp();
 assert!(WORLD.lock().unwrap().as_ref().unwrap().accepted.iter().all(|e|matches!(e,Envelope::Dns(..))));
}
#[test]fn disallowed_ip_zero_ttl_expiry_replacement_and_foreign_task_admit_no_tcp(){
 let _serial=SERIAL.lock().unwrap();
 for case in 0..5{
  reset(false,if case==0{[203,0,113,9]}else{[10,0,2,2]},if case==1{0}else if case==2{1}else{60});
  let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();dns.start();let mut now=finish_dns(&mut dns,&mut http,1);let generation=dns.generation().unwrap();
  if case==2{now=WORLD.lock().unwrap().as_ref().unwrap().dns.resolved(A,generation,now).unwrap().expires_tick;}
  if case==3{dns.start();now=finish_dns(&mut dns,&mut http,now+1);assert_ne!(dns.generation(),Some(generation));}
  if case==4{WORLD.lock().unwrap().as_mut().unwrap().caller=B;}
  assert!(http.start_resolved(1,generation));finish_http(&mut dns,&mut http,now+1);
  assert_eq!(http.view(),http_probe::View::Unavailable,"case={case}");assert_eq!(http.preview(),&[b' ';64]);assert!(http.raw_scrubbed());assert_no_tcp();
 }
}
#[test]fn accepted_begin_duplicate_replays_after_expiry_and_copied_admission_finishes(){
 let _serial=SERIAL.lock().unwrap();reset(false,[10,0,2,2],1);let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();dns.start();let now=finish_dns(&mut dns,&mut http,1);let generation=dns.generation().unwrap();
 let expiry=WORLD.lock().unwrap().as_ref().unwrap().dns.resolved(A,generation,now).unwrap().expires_tick;
 assert!(http.start_resolved(1,generation));http.advance(now+1);pump(&mut dns,&mut http,now+1);
 let original=WORLD.lock().unwrap().as_ref().unwrap().tcp.resolution().unwrap();assert_eq!(service_call(A,request(1,generation),expiry+1),Ok(1));
 {let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.dns.resolved(A,generation,expiry+1),None);assert_eq!(w.tcp.resolution(),Some(original));assert!(w.http_wire.begins.is_empty());}
 finish_http(&mut dns,&mut http,expiry+2);assert_eq!(http.view(),http_probe::View::Ready);assert!(http.raw_scrubbed());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().http_wire.begins,vec![1]);
}
#[test]fn denied_begin_duplicate_stays_denied_after_actual_dns_renewal_then_new_sequence_succeeds(){
 let _serial=SERIAL.lock().unwrap();reset(false,[10,0,2,2],60);let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();assert!(http.start_resolved(10,1));finish_http(&mut dns,&mut http,1);
 assert_eq!(http.view(),http_probe::View::Unavailable);assert_no_tcp();dns.start();let now=finish_dns(&mut dns,&mut http,100);assert_eq!(dns.generation(),Some(1));
 assert_eq!(service_call(A,request(10,1),now+1),Err(network_job::DENIED));assert_no_tcp();
 assert!(http.start_resolved(http.next_sequence(),1));finish_http(&mut dns,&mut http,now+2);assert_eq!(http.view(),http_probe::View::Ready);assert!(http.raw_scrubbed());
}
#[test]fn late_accepted_resolved_begin_cancel_retains_exact_correlation_and_only_cleans(){
 let _serial=SERIAL.lock().unwrap();reset(false,[10,0,2,2],60);let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();dns.start();let now=finish_dns(&mut dns,&mut http,1);let generation=dns.generation().unwrap();
 assert!(http.start_resolved(50,generation));http.advance(now+1);let accepted=WORLD.lock().unwrap().as_ref().unwrap().accepted.clone();
 http.advance(now+2001);assert_eq!(http.view(),http_probe::View::Uncertain);http.cancel();http.advance(now+2002);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted,accepted);
 assert!(!http.start_resolved(100,generation));pump(&mut dns,&mut http,now+2003);finish_http(&mut dns,&mut http,now+2004);
 assert_eq!(http.view(),http_probe::View::Cancelled);assert!(http.raw_scrubbed());assert_eq!(http.preview(),&[b' ';64]);assert_no_tcp();
 let guard=WORLD.lock().unwrap();let requests=guard.as_ref().unwrap().accepted.iter().filter_map(|e|if let Envelope::Tcp(_,r)=e{Some(r.kind)}else{None}).collect::<Vec<_>>();assert_eq!(requests,vec![network_job::BEGIN_STREAM_RESOLVED,network_job::CANCEL,network_job::RELEASE]);
}
#[test]fn exact_released_record_retirement_blocks_handoff_and_foreign_task_cannot_clear_it(){
 let _serial=SERIAL.lock().unwrap();reset(false,[10,0,2,2],60);let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();dns.start();let now=finish_dns(&mut dns,&mut http,1);let generation=dns.generation().unwrap();
 {let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();assert_eq!(w.dns.owner(),None);assert_eq!(w.dns.owned_records(A),1);assert!(!w.dns.retire(B,&mut w.dns_wire));assert_eq!(w.dns.owned_records(A),1);assert!(!w.dns.retire(A,&mut w.dns_wire));assert_eq!(w.dns.owned_records(A),0);}
 assert!(http.start_resolved(1,generation));finish_http(&mut dns,&mut http,now+1);assert_eq!(http.view(),http_probe::View::Unavailable);assert_no_tcp();
}

#[test]
fn twelve_kib_stream_wraps_private_ring_and_waits_for_fin_and_positive_release() {
 let _serial=SERIAL.lock().unwrap();reset(true,[10,0,2,2],60);
 let body:Vec<u8>=(0..12288).map(|n|b'A'+(n%26)as u8).collect();
 {let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();w.http_wire.body=body.clone();w.http_wire.allow_fin=false;}
 let mut dns=dns_probe::Client::new();let mut http=http_probe::Client::new();dns.start();let now=finish_dns(&mut dns,&mut http,1);
 assert!(http.start_resolved(1,dns.generation().unwrap()));
 let mut drained=None;
 for tick in now+1..now+15000 {
  http.advance(tick);pump(&mut dns,&mut http,tick);
  let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();
  let tail=w.accepted.iter().rev().find_map(|e|if let Envelope::Tcp(_,r)=e{Some(r)}else{None});
  if w.http_wire.read==w.http_wire.response.len() && tail.is_some_and(|r|r.kind==network_job::READ && r.offset as usize+r.length as usize==w.http_wire.response.len()) {
   drained=Some(tick);break;
  }
 }
 let now=drained.expect("12KiB did not drain through bounded streaming ring");
 assert_eq!(http.view(),http_probe::View::Pending);assert!(http.document().is_empty());assert_eq!(http.body_length(),0);
 {let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();assert_eq!(w.tcp.snapshot().state,network_job::State::Receiving);assert!(w.http_wire.response.len()>network_job::MAX_BYTES);w.http_wire.link=jobs::Link::PeerFin;}
 let now=wait_http_release(&mut dns,&mut http,now+1);assert_eq!(http.view(),http_probe::View::Pending);assert!(http.document().is_empty());
 pump(&mut dns,&mut http,now);assert_eq!(http.view(),http_probe::View::Ready);assert_eq!(http.document(),body);assert!(http.raw_scrubbed());
 let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.tcp.owner().is_none());
 assert!(w.accepted.iter().any(|e|matches!(e,Envelope::Tcp(_,r)if r.kind==network_job::BEGIN_STREAM_RESOLVED)));
 assert!(w.accepted.iter().any(|e|matches!(e,Envelope::Tcp(_,r)if r.kind==network_job::READ && r.offset as usize>network_job::MAX_BYTES)));
}
