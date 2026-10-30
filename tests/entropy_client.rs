// SPDX-License-Identifier: GPL-3.0-only
//! Actual client/ABIs/private broker. Provider values are public test fixtures;
//! there is no live CPU entropy source, guest pointer, hash or serial output.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../servers/sexnet/src/entropy.rs"]mod broker;
#[path="../apps/kaleidoscope/src/entropy_probe.rs"]mod probe;
use std::sync::Mutex;
use entropy_seed::{self as a,Request,Identity,State};
use entropy_source::Info;
pub const SLOT_NET:u64=18;
pub const ERR_SERVICE_NOT_READY:u64=9;
const OWNER:Identity=Identity{pd:15,task:150};
const INFO:Option<Info>=Some(Info{consumer:OWNER,available:true});
const FIXTURE:[u8;32]=[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31];
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
struct World{
    machine:broker::Machine,request:Option<Request>,accepted:Vec<Request>,calls:usize,
    full_each:bool,last_full:u32,full_kind:Option<u8>,hard_kind:Option<u8>,
    hold_kind:Option<u8>,held:Option<(Request,u64)>,reject_release:bool,
    info:Option<Info>,owner:Identity,provider_calls:usize,source_error:Option<u64>,
    status_override:Option<u32>,provider_enabled:bool,
}
fn reset(full_each:bool){*WORLD.lock().unwrap()=Some(World{
    machine:broker::Machine::new(),request:None,accepted:Vec::new(),calls:0,
    full_each,last_full:0,full_kind:None,hard_kind:None,hold_kind:None,held:None,reject_release:false,
    info:INFO,owner:OWNER,provider_calls:0,source_error:None,status_override:None,provider_enabled:true,
});}
pub fn pdx_call(slot:u64,op:u64,x:u64,y:u64,z:u64)->(u64,u64){
    assert!(slot==SLOT_NET&&op==a::OP);let r=Request::decode([x,y,z]).unwrap();assert!(r.valid());
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.calls+=1;
    if w.full_kind==Some(r.kind)||(w.full_each&&w.last_full!=r.seq){w.last_full=r.seq;return(ERR_SERVICE_NOT_READY,0);}
    if w.hard_kind==Some(r.kind){return(u64::MAX,0);}
    assert!(w.request.is_none());assert!(w.accepted.last().is_none_or(|old|old.seq<r.seq),"accepted mutation replayed");
    w.accepted.push(r);w.request=Some(r);(0,0)
}
fn service(now:u64)->Option<u64>{
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();let r=w.request.take();
    if let Some(r)=r{
        if r.kind==a::RELEASE&&w.reject_release{return Some(a::error(r.seq,a::IO));}
        assert!(w.machine.handle(w.owner,r,now,w.info));
    }
    if w.provider_enabled{
        let calls=&mut w.provider_calls;let error=w.source_error;let at=w.machine.snapshot().produced;
        w.machine.advance(now,||{
            *calls+=1;if let Some(error)=error{return Err(error);}
            assert!(at<32&&at%8==0,"source sampled beyond current job");
            Ok(u64::from_le_bytes(FIXTURE[at..at+8].try_into().unwrap()))
        });
    }
    let result=w.machine.pending_reply().map(|(owner,word)|{assert!(owner==w.owner);word});
    if result.is_some(){w.machine.reply_accepted();}
    let mut result=result;
    if r.is_some_and(|r|r.kind==a::STATUS){if let Some(value)=w.status_override.take(){result=Some(a::reply(r.unwrap().seq,value));}}
    if r.is_some_and(|r|w.hold_kind==Some(r.kind))&&result.is_some(){assert!(w.held.is_none());w.held=Some((r.unwrap(),result.unwrap()));w.hold_kind=None;None}else{result}
}
fn pump(c:&mut probe::Client,now:u64){if let Some(word)=service(now){c.reply(word,now);}}
fn turn(c:&mut probe::Client,now:u64){c.advance(now);pump(c,now);}
fn finish(c:&mut probe::Client,start:u64){for now in start..start+5000{turn(c,now);if !c.busy(){return;}}panic!("client did not clean up");}
fn hold(c:&mut probe::Client,kind:u8,start:u64)->u64{
    WORLD.lock().unwrap().as_mut().unwrap().hold_kind=Some(kind);
    for now in start..start+1000{turn(c,now);if WORLD.lock().unwrap().as_ref().unwrap().held.is_some(){return now;}}
    panic!("requested accepted phase was not reached");
}
fn deliver(c:&mut probe::Client,now:u64){let(_,word)=WORLD.lock().unwrap().as_mut().unwrap().held.take().unwrap();c.reply(word,now);}

#[test]fn initial_idle_and_explicit_start_have_no_automatic_pdx(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();assert!(c.view()==probe::View::Idle&&!c.busy());
    for now in[0,2000,u64::MAX]{c.advance(now);c.reply(a::reply(1,1),now);}assert!(c.take_seed().is_none());assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==0);
    assert!(c.start());assert!(!c.start());assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==0);c.cancel();assert!(!c.busy()&&c.private_scrubbed());c.advance(1);assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==0);
}
#[test]fn actual_broker_all_enqueue_full_once_then_release_and_take_exactly_once(){
    let _s=SERIAL.lock().unwrap();reset(true);let mut c=probe::Client::new();assert!(c.start());finish(&mut c,1);
    assert!(c.view()==probe::View::Ready&&!c.busy());let seed=c.take_seed().unwrap();assert!(seed==FIXTURE);assert!(c.take_seed().is_none()&&c.private_scrubbed()&&c.view()==probe::View::Idle);
    let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none()&&w.machine.private_scrubbed()&&w.provider_calls==4&&w.calls>w.accepted.len());
    let reads:Vec<_>=w.accepted.iter().filter(|r|r.kind==a::READ).collect();assert!(reads.len()==8);assert!(reads.iter().enumerate().all(|(n,r)|r.offset==n as u16*4&&r.length==4));
}
#[test]fn no_seed_is_exposed_before_exact_positive_release_receipt(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::RELEASE,1);
    assert!(c.busy()&&c.view()==probe::View::Pending&&c.take_seed().is_none());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
    deliver(&mut c,now+1);assert!(!c.busy()&&c.view()==probe::View::Ready);assert!(c.take_seed().is_some()&&c.private_scrubbed());
}
#[test]fn explicit_fresh_start_keeps_sequence_and_ticket_progress_without_automatic_retry(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();finish(&mut c,1);assert!(c.take_seed().unwrap()==FIXTURE);
    let watermark=c.next_sequence();let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;for now in 100..110{turn(&mut c,now);}assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==calls);
    assert!(c.start());assert!(c.next_sequence()==watermark);finish(&mut c,110);assert!(c.take_seed().unwrap()==FIXTURE);
    let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.provider_calls==8);let begins:Vec<_>=w.accepted.iter().filter(|r|r.kind==a::BEGIN).collect();assert!(begins.len()==2&&begins[1].seq==watermark);let releases:Vec<_>=w.accepted.iter().filter(|r|r.kind==a::RELEASE).collect();assert!(releases.len()==2&&releases[0].ticket==1&&releases[1].ticket==2);
}
#[test]fn wrong_tags_sequences_and_stale_read_do_not_consume_accepted_correlation(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::READ,1);let r=WORLD.lock().unwrap().as_ref().unwrap().held.unwrap().0;let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;
    for word in[(0x78u64<<56)|((r.seq as u64)<<32)|19,(0x79u64<<56)|((r.seq as u64)<<32)|19,a::reply(r.seq+1,19),a::reply(r.seq-1,19)]{c.reply(word,now);}
    c.advance(now+1);assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==calls&&c.take_seed().is_none());deliver(&mut c,now+2);finish(&mut c,now+3);assert!(c.take_seed().unwrap()==FIXTURE);
}
#[test]fn late_accepted_begin_timeout_retains_correlation_and_never_replays(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::BEGIN,1);let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;
    c.advance(now+2000);assert!(c.busy()&&c.view()==probe::View::Uncertain&&c.private_scrubbed());assert!(!c.start());for at in now+2001..now+2005{c.advance(at);}assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==calls);
    deliver(&mut c,now+2005);finish(&mut c,now+2006);assert!(!c.busy()&&c.view()==probe::View::Unavailable&&c.take_seed().is_none());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.private_scrubbed());
}
#[test]fn explicit_cancel_at_late_begin_read_and_release_cannot_publish(){
    let _s=SERIAL.lock().unwrap();for kind in[a::BEGIN,a::READ,a::RELEASE]{reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,kind,1);c.cancel();assert!(c.private_scrubbed()&&c.take_seed().is_none()&&c.busy());deliver(&mut c,now+1);finish(&mut c,now+2);assert!(!c.busy()&&c.view()==probe::View::Cancelled&&c.private_scrubbed()&&c.take_seed().is_none());let g=WORLD.lock().unwrap();assert!(g.as_ref().unwrap().machine.owner().is_none()&&g.as_ref().unwrap().machine.private_scrubbed());}
}
#[test]fn cancelled_full_begin_is_known_preeffect_and_no_job_is_allocated(){
    let _s=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().full_kind=Some(a::BEGIN);let mut c=probe::Client::new();c.start();c.advance(1);c.cancel();c.advance(2);assert!(!c.busy()&&c.view()==probe::View::Cancelled&&c.private_scrubbed());let g=WORLD.lock().unwrap();assert!(g.as_ref().unwrap().accepted.is_empty()&&g.as_ref().unwrap().machine.owner().is_none());
}
#[test]fn permanently_full_begin_has_bounded_retry_then_requires_new_user_intent(){
    let _s=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().full_kind=Some(a::BEGIN);let mut c=probe::Client::new();c.start();c.advance(1);c.advance(2001);assert!(!c.busy()&&c.view()==probe::View::Unavailable&&c.private_scrubbed());let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;c.advance(2002);assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==calls);
}
#[test]fn unsupported_wrong_task_and_source_failure_never_expose_a_seed(){
    let _s=SERIAL.lock().unwrap();for mode in 0..3{reset(false);{let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();match mode{0=>w.info=Some(Info{consumer:OWNER,available:false}),1=>w.owner=Identity{pd:15,task:151},_=>w.source_error=Some(entropy_source::BUSY)}}let mut c=probe::Client::new();c.start();finish(&mut c,1);assert!(!c.busy()&&c.view()==probe::View::Unavailable&&c.private_scrubbed()&&c.take_seed().is_none());let g=WORLD.lock().unwrap();let w=g.as_ref().unwrap();assert!(w.machine.owner().is_none()&&w.machine.private_scrubbed());assert!(w.provider_calls==if mode==2{1}else{0});}
}
#[test]fn malformed_status_state_length_or_progress_cannot_become_readable(){
    let _s=SERIAL.lock().unwrap();for value in[0,(1<<24)|32,(1<<24)|7,(2<<24)|31,(2<<24)|33,(3<<24)|32]{reset(false);WORLD.lock().unwrap().as_mut().unwrap().status_override=Some(value);let mut c=probe::Client::new();c.start();finish(&mut c,1);assert!(c.view()==probe::View::Unavailable&&c.private_scrubbed()&&c.take_seed().is_none());assert!(WORLD.lock().unwrap().as_ref().unwrap().accepted.iter().all(|r|r.kind!=a::READ));}
}
#[test]fn collecting_progress_regression_fails_without_seed_publication(){
    let _s=SERIAL.lock().unwrap();reset(false);{let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();w.provider_enabled=false;w.status_override=Some(a::status(State::Collecting,16));}
    let mut c=probe::Client::new();c.start();for now in 1..4{turn(&mut c,now);}WORLD.lock().unwrap().as_mut().unwrap().status_override=Some(a::status(State::Collecting,8));finish(&mut c,30);assert!(c.view()==probe::View::Unavailable&&c.private_scrubbed()&&c.take_seed().is_none());
}
#[test]fn failed_release_retains_custody_and_explicit_cancel_can_clean_without_restart(){
    let _s=SERIAL.lock().unwrap();reset(false);WORLD.lock().unwrap().as_mut().unwrap().reject_release=true;let mut c=probe::Client::new();c.start();for now in 1..100{turn(&mut c,now);if c.view()==probe::View::Uncertain{break;}}
    assert!(c.busy()&&c.view()==probe::View::Uncertain&&c.private_scrubbed()&&c.take_seed().is_none());assert!(!c.start());let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;for now in 101..110{c.advance(now);}assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==calls);
    WORLD.lock().unwrap().as_mut().unwrap().reject_release=false;c.cancel();finish(&mut c,111);assert!(!c.busy()&&c.view()==probe::View::Cancelled&&c.take_seed().is_none());assert!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner().is_none());
}
#[test]fn full_read_cancel_abandons_only_unaccepted_request_and_scrubs_partial_bytes(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::READ,1);deliver(&mut c,now+1);assert!(!c.private_scrubbed());WORLD.lock().unwrap().as_mut().unwrap().full_kind=Some(a::READ);c.advance(now+2);c.cancel();assert!(c.private_scrubbed());finish(&mut c,now+3);assert!(c.view()==probe::View::Cancelled&&WORLD.lock().unwrap().as_ref().unwrap().machine.private_scrubbed());
}
#[test]fn service_cached_read_replay_is_idempotent_and_client_does_not_duplicate_it(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::READ,1);
    {let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();let(r,word)=w.held.unwrap();let before=w.machine.snapshot();assert!(w.machine.handle(OWNER,r,now,INFO));assert!(w.machine.pending_reply().unwrap().1==word&&w.machine.snapshot()==before);w.machine.reply_accepted();}
    deliver(&mut c,now+1);finish(&mut c,now+2);assert!(c.take_seed().unwrap()==FIXTURE);
}
#[test]fn terminal_close_stops_local_work_but_exact_retire_is_service_cleanup(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();let now=hold(&mut c,a::READ,1);let calls=WORLD.lock().unwrap().as_ref().unwrap().calls;c.close();assert!(!c.busy()&&c.private_scrubbed()&&c.view()==probe::View::Cancelled&&!c.start());deliver(&mut c,now+1);c.advance(now+2);assert!(c.take_seed().is_none()&&WORLD.lock().unwrap().as_ref().unwrap().calls==calls);let mut g=WORLD.lock().unwrap();let w=g.as_mut().unwrap();assert!(w.machine.owner()==Some(OWNER));assert!(w.machine.retire(OWNER)&&w.machine.private_scrubbed());
}
#[test]fn backward_clock_and_deadline_overflow_cannot_publish_or_forget_an_accepted_begin(){
    let _s=SERIAL.lock().unwrap();reset(false);let mut c=probe::Client::new();c.start();c.advance(u64::MAX);assert!(!c.busy()&&c.private_scrubbed()&&c.view()==probe::View::Unavailable);assert!(WORLD.lock().unwrap().as_ref().unwrap().calls==0);
    reset(false);let mut c=probe::Client::new();c.start();hold(&mut c,a::BEGIN,10);c.advance(9);assert!(c.busy()&&c.view()==probe::View::Uncertain);deliver(&mut c,11);finish(&mut c,12);assert!(c.view()==probe::View::Unavailable&&c.take_seed().is_none());
}
