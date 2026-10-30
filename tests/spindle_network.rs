// SPDX-License-Identifier: GPL-3.0-only
//! Actual Spindle wrapper, shared client, ABI and service machine. The wire is
//! an explicit partial-I/O peer; it provides no caller authority or DNS policy.
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/network_job.rs"] pub mod network_job;
#[path = "../servers/sexnet/src/jobs.rs"] mod jobs;
#[path = "../apps/spindle/src/network_probe.rs"] mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;
pub const SLOT_DISPLAY:u64=5;
pub const ERR_SERVICE_NOT_READY:u64=9;
#[macro_export] macro_rules! serial_record { ($($t:tt)*) => { let _=std::format!($($t)*); }; }
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
const OWNER:jobs::Identity=jobs::Identity{pd:12,task:120};
const FOREIGN:jobs::Identity=jobs::Identity{pd:15,task:150};
struct Peer { link:jobs::Link, bytes:Vec<u8>, at:usize, begins:usize, aborts:usize }
impl jobs::Wire for Peer {
    fn begin(&mut self,_:u32)->Result<(),u32>{self.link=jobs::Link::Open;self.bytes.clear();self.at=0;self.begins+=1;Ok(())}
    fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{let n=out.len().min(11).min(self.bytes.len()-self.at);out[..n].copy_from_slice(&self.bytes[self.at..self.at+n]);self.at+=n;Ok(n)}
    fn send(&mut self,data:&[u8])->Result<usize,u32>{let n=data.len().min(7);self.bytes.extend_from_slice(&data[..n]);Ok(n)}
    fn queued(&self)->usize{0}
    fn link(&self)->jobs::Link{self.link}
    fn finish_send(&mut self){self.link=jobs::Link::PeerFin;}
    fn abort(&mut self){self.link=jobs::Link::Closed;self.bytes.clear();self.at=0;self.aborts+=1;}
}
struct World {
    machine:jobs::Machine, wire:Peer, request:Option<network_job::Request>,
    accepted:Vec<network_job::Request>, calls:usize, full_each:bool, last_full:u32,
    held_reply:Option<u64>, delay_release:bool,
}
fn reset(full_each:bool) {
    *WORLD.lock().unwrap()=Some(World{machine:jobs::Machine::new(),wire:Peer{link:jobs::Link::Closed,bytes:Vec::new(),at:0,begins:0,aborts:0},request:None,accepted:Vec::new(),calls:0,full_each,last_full:0,held_reply:None,delay_release:false});
}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64) {
    assert_eq!(slot,SLOT_NET);assert_eq!(op,network_job::OP);
    let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.calls+=1;
    if w.full_each&&w.last_full!=r.seq{w.last_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
    assert!(w.request.is_none());assert!(w.accepted.iter().all(|old|old.seq!=r.seq),"accepted mutation replayed");
    w.accepted.push(r);w.request=Some(r);(0,0)
}
fn service(now:u64)->Option<u64> {
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();
    let mut delayed=false;
    if let Some(r)=w.request.take(){assert!(w.machine.handle(OWNER,r,now,&mut w.wire));delayed=w.delay_release&&r.kind==network_job::RELEASE;}
    w.machine.advance(now,&mut w.wire);
    if let Some((owner,word))=w.machine.pending_reply(){assert_eq!(owner,OWNER);w.machine.reply_accepted();if delayed{assert!(w.held_reply.is_none());w.held_reply=Some(word);None}else{Some(word)}}else{None}
}
fn turn(p:&mut probe::Client,now:u64){p.advance(now);if let Some(word)=service(now){p.reply(word,now);}}
fn finish(p:&mut probe::Client,start:u64){for now in start..start+6000{turn(p,now);if !p.busy(){return;}}panic!("client failed to finish: {:?}",p.view());}

#[test] fn initial_idle_and_status_have_no_pdx_effects() {
    let _s=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();
    assert_eq!(p.view(),probe::View::Idle);assert_eq!(p.status(),"idle");assert!(!p.busy());assert!(!p.cancel());
    for now in [0,1,2001,u64::MAX]{p.advance(now);p.reply(network_job::reply(1,1),now);}
    assert_eq!(p.status_generation(),0);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
}
#[test] fn actual_partial_echo_full_each_enqueue_then_positive_release() {
    let _s=SERIAL.lock().unwrap();reset(true);let mut p=probe::Client::new();assert!(p.start_or_retry());finish(&mut p,1);
    assert_eq!(p.view(),probe::View::Verified);assert!(!p.busy());assert!(p.status_generation()>=2);
    let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none());assert_eq!(w.wire.begins,1);assert!(w.calls>w.accepted.len());assert!(w.accepted.len()>350);
}
#[test] fn verified_is_hidden_until_positive_release_receipt() {
    let _s=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().delay_release=true;
    let mut p=probe::Client::new();p.start_or_retry();
    for now in 1..5000{turn(&mut p,now);if WORLD.lock().unwrap().as_ref().unwrap().held_reply.is_some(){break;}}
    assert!(p.busy());assert_eq!(p.view(),probe::View::Pending);
    let word=WORLD.lock().unwrap().as_mut().unwrap().held_reply.take().unwrap();p.reply(word,5000);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Verified);
}
#[test] fn foreign_held_job_returns_busy_then_explicit_retry_uses_new_sequence() {
    let _s=SERIAL.lock().unwrap();reset(false);
    {let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();let r=network_job::Request{seq:1,kind:network_job::BEGIN,offset:0,length:1,ticket:0,a:0};assert!(w.machine.handle(FOREIGN,r,0,&mut w.wire));w.machine.reply_accepted();}
    let mut p=probe::Client::new();p.start_or_retry();finish(&mut p,1);assert_eq!(p.view(),probe::View::Unavailable);
    let watermark=p.next_sequence();{let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();assert!(w.machine.retire(FOREIGN,&mut w.wire));assert_eq!(w.wire.begins,0);}
    for now in 20..30{turn(&mut p,now);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.len(),1);
    assert!(p.start_or_retry());finish(&mut p,30);assert_eq!(p.view(),probe::View::Verified);
    let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.accepted[1].seq>=watermark);assert_eq!(w.wire.begins,1);
}
#[test] fn late_accepted_begin_cancel_preserves_exact_correlation_no_replay() {
    let _s=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start_or_retry();p.advance(1);
    p.reply((0x79u64<<56)|(1<<32)|1,2);p.reply(network_job::reply(2,1),2);p.advance(2002);
    assert_eq!(p.view(),probe::View::Uncertain);assert!(p.busy());assert!(p.cancel());
    for now in 2003..2010{p.advance(now);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);
    finish(&mut p,2010);assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());
    let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none());assert_eq!(w.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),[network_job::BEGIN,network_job::CANCEL,network_job::RELEASE]);assert_eq!(w.wire.begins,0);
}
#[test] fn last_cancel_overrides_queued_retry_and_does_not_start_fresh_job() {
    let _s=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start_or_retry();p.advance(1);p.start_or_retry();p.cancel();finish(&mut p,2);
    assert_eq!(p.view(),probe::View::Cancelled);let seq=p.next_sequence();for now in 100..200{turn(&mut p,now);}assert_eq!(p.next_sequence(),seq);
    let g=WORLD.lock().unwrap();assert_eq!(g.as_ref().unwrap().accepted.len(),3);assert!(g.as_ref().unwrap().machine.owner().is_none());
}
#[test] fn accepted_release_error_retains_uncertain_custody_until_explicit_cleanup() {
    let _s=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().delay_release=true;
    let mut p=probe::Client::new();p.start_or_retry();for now in 1..5000{turn(&mut p,now);if WORLD.lock().unwrap().as_ref().unwrap().held_reply.is_some(){break;}}
    let seq=WORLD.lock().unwrap().as_ref().unwrap().accepted.last().unwrap().seq;
    // A service error is not positive cleanup, even if data validated earlier.
    p.reply(network_job::error(seq,network_job::IO),5000);assert_eq!(p.view(),probe::View::Uncertain);assert!(p.busy());
    for now in 5001..5010{p.advance(now);}let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;
    p.advance(9000);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
}
#[test] fn terminal_close_stops_local_work_service_retirement_remains_separate() {
    let _s=SERIAL.lock().unwrap();reset(false);let mut p=probe::Client::new();p.start_or_retry();p.advance(1);let word=service(1).unwrap();p.reply(word,1);
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner(),Some(OWNER));p.close();assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.start_or_retry());
    let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;for now in 2..20{p.advance(now);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,calls);
    let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();assert!(w.machine.retire(OWNER,&mut w.wire));assert!(w.machine.owner().is_none());
}
