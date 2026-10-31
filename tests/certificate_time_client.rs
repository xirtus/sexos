// SPDX-License-Identifier: GPL-3.0-only
//! Actual ABI, source validator, service machine and application client.
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
mod app_catalog;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/certificate_time.rs"]pub mod certificate_time;
#[path="../servers/sexnet/src/certificate_time.rs"]mod broker;
#[path="../apps/kaleidoscope/src/clock_probe.rs"]mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
const OWNER:clock_source::Identity=clock_source::Identity {pd:15,task:15};
const T:u64=clock_source::MIN_SECONDS+100;
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
struct World {machine:broker::Machine,request:Option<certificate_time::Request>,calls:usize,samples:usize,
    source:Result<clock_source::Snapshot,u64>,full:usize,reject:bool,accepted:Vec<u32>}
fn reset() {*WORLD.lock().unwrap()=Some(World {machine:broker::Machine::new(),request:None,calls:0,samples:0,
    source:Ok(clock_source::Snapshot {seconds:T,attempts:1}),full:0,reject:false,accepted:vec![]});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64) {
    assert_eq!((slot,op),(SLOT_NET,certificate_time::OP));let r=certificate_time::Request::decode([a,b,c]).unwrap();
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.calls+=1;
    if w.full!=0 {w.full-=1;return (ERR_SERVICE_NOT_READY,0);}
    if w.reject {return (123,0);}
    assert!(w.request.is_none());assert!(w.accepted.last().is_none_or(|seq|r.seq>*seq),"accepted query replayed");
    w.accepted.push(r.seq);w.request=Some(r);(0,0)
}
fn receipt()->u64 {
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();let r=w.request.take().unwrap();
    let source=w.source;let calls=&mut w.samples;
    assert!(w.machine.handle(OWNER,r,Some(clock_source::Info {consumer:OWNER,source:clock_source::CMOS_UTC}),||{*calls+=1;source}));
    let (owner,word)=w.machine.pending_reply().unwrap();assert_eq!(owner,OWNER);w.machine.reply_accepted();word
}
fn deliver(p:&mut probe::Client,now:u64) {p.reply(receipt(),now);}
#[test] fn explicit_fresh_queries_publish_once_and_raise_service_highwater() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();p.advance(0);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
    assert_eq!(p.view(),probe::View::Idle);assert!(p.start());assert!(!p.start());assert_eq!(p.take_snapshot(),None);p.advance(10);p.advance(11);
    assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);deliver(&mut p,12);assert_eq!(p.view(),probe::View::Ready(T as u32));assert_eq!(p.take_snapshot(),Some(T as u32));assert_eq!(p.take_snapshot(),None);
    WORLD.lock().unwrap().as_mut().unwrap().source=Ok(clock_source::Snapshot {seconds:T+1,attempts:2});
    assert!(p.start());assert_eq!(p.take_snapshot(),None);p.advance(20);deliver(&mut p,21);assert_eq!(p.take_snapshot(),Some(T as u32+1));
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.accepted,vec![1,2]);assert_eq!(w.samples,2);assert_eq!(w.machine.snapshot().high_water,Some(T+1));
}
#[test] fn only_full_retries_are_bounded_and_preserve_one_query_sequence() {
    let _serial=SERIAL.lock().unwrap();reset();WORLD.lock().unwrap().as_mut().unwrap().full=2;
    let mut p=probe::Client::new();assert!(p.start());for now in 10..14 {p.advance(now);}deliver(&mut p,14);
    assert_eq!(p.view(),probe::View::Ready(T as u32));let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.calls,3);assert_eq!(w.accepted,vec![1]);assert_eq!(w.samples,1);drop(guard);
    assert!(p.start());WORLD.lock().unwrap().as_mut().unwrap().full=usize::MAX;p.advance(100);p.advance(2100);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_snapshot(),None);assert_eq!(p.next_sequence(),3);
}
#[test] fn cancellation_before_effect_consumes_sequence_without_source_or_automatic_retry() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();assert!(p.start());p.cancel();p.advance(1);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);assert_eq!(p.next_sequence(),2);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,0);
    assert!(p.start());WORLD.lock().unwrap().as_mut().unwrap().full=1;p.advance(2);p.cancel();p.advance(3);
    assert_eq!(p.next_sequence(),3);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().samples,0);assert!(p.start());p.advance(4);deliver(&mut p,5);assert_eq!(p.take_snapshot(),Some(T as u32));
}
#[test] fn accepted_cancel_keeps_correlation_until_actual_ack_and_never_publishes_late_success() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();assert!(p.start());p.advance(1);p.cancel();assert!(p.busy());assert!(!p.start());
    for now in [2,2001,3000] {p.advance(now);}assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);deliver(&mut p,3001);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);assert_eq!(p.take_snapshot(),None);assert!(p.start());
}
#[test] fn accepted_timeout_and_wrong_tag_sequence_never_resend_or_become_ready() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();assert!(p.start());p.advance(100);
    p.reply((0x7Au64<<56)|(1<<32)|T,101);p.reply(certificate_time::reply(2,T as u32),102);assert!(p.busy());
    p.advance(2100);assert_eq!(p.view(),probe::View::Uncertain);assert!(!p.start());p.advance(2101);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);
    deliver(&mut p,2102);assert!(!p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_snapshot(),None);assert!(p.start());
}
#[test] fn reply_itself_enforces_deadline_and_backwards_time_even_without_timeout_turn() {
    let _serial=SERIAL.lock().unwrap();for now in [2100,99] {reset();let mut p=probe::Client::new();assert!(p.start());p.advance(100);deliver(&mut p,now);assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_snapshot(),None);assert!(!p.busy());}
}
#[test] fn offline_invalid_and_rejected_queries_never_fall_back_to_last_sample() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();assert!(p.start());p.advance(1);deliver(&mut p,2);
    assert!(p.start());WORLD.lock().unwrap().as_mut().unwrap().source=Err(clock_source::UNAVAILABLE);p.advance(3);deliver(&mut p,4);assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_snapshot(),None);
    assert!(p.start());p.advance(5);let _actual=receipt();p.reply((0x7Bu64<<56)|(3<<32),6);assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_snapshot(),None);
    assert!(p.start());WORLD.lock().unwrap().as_mut().unwrap().reject=true;p.advance(7);p.advance(8);assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,4);
}
#[test] fn actual_broker_rollback_is_permanent_even_for_cancelled_or_late_query() {
    let _serial=SERIAL.lock().unwrap();for cancelled in [false,true] {reset();let mut p=probe::Client::new();assert!(p.start());p.advance(1);deliver(&mut p,2);
        assert!(p.start());WORLD.lock().unwrap().as_mut().unwrap().source=Ok(clock_source::Snapshot {seconds:T-1,attempts:1});p.advance(3);if cancelled {p.cancel();}else{p.advance(2003);}
        deliver(&mut p,2004);assert_eq!(p.view(),probe::View::Rollback);assert_eq!(p.take_snapshot(),None);assert!(!p.start());p.cancel();assert_eq!(p.view(),probe::View::Rollback);
        let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.samples,2);assert!(w.machine.snapshot().rollback);
    }
}
#[test] fn close_stops_local_work_and_exact_service_retirement_discards_owner_receipts() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();assert!(p.start());p.advance(1);let word=receipt();p.close();p.reply(word,2);p.advance(3);
    assert!(!p.busy());assert!(!p.start());assert_eq!(p.take_snapshot(),None);assert_eq!(p.view(),probe::View::Cancelled);
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();assert!(w.machine.retire(OWNER));assert_eq!(w.machine.snapshot().high_water,Some(T));assert_eq!(w.calls,1);
}
#[test] fn finite_last_sequence_completes_then_stops_without_wrap_or_source_call() {
    let _serial=SERIAL.lock().unwrap();reset();let mut p=probe::Client::new();p.set_next_sequence_for_test(certificate_time::MAX_SEQ);
    assert!(p.start());p.advance(1);deliver(&mut p,2);assert_eq!(p.take_snapshot(),Some(T as u32));assert_eq!(p.next_sequence(),certificate_time::MAX_SEQ+1);assert!(!p.start());p.advance(3);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls,1);assert_eq!(p.view(),probe::View::Unavailable);
}

mod clock_probe{pub use super::probe::View;}
#[path="../apps/kaleidoscope/src/clock_view.rs"]mod clock_view;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[test]fn utc_view_formats_real_calendar_bounds_and_stale_states_as_public_data(){
 for (seconds,expected)in[(clock_source::MIN_SECONDS as u32,"2020-01-01T00:00:00Z"),(clock_source::MAX_SECONDS as u32,"2099-12-31T23:59:59Z"),(1582934400,"2020-02-29T00:00:00Z"),(1793367907,"2026-10-30T13:45:07Z")]{let s=clock_view::status(probe::View::Ready(seconds)).unwrap();assert_eq!(&s.bytes[..s.len],expected.as_bytes());}
 for (view,expected)in[(probe::View::Pending,"checking"),(probe::View::Unavailable,"unavailable"),(probe::View::Cancelled,"cancelled"),(probe::View::Uncertain,"waiting receipt"),(probe::View::Rollback,"rollback; blocked"),(probe::View::Ready(0),"unavailable")]{let s=clock_view::status(view).unwrap();assert_eq!(&s.bytes[..s.len],expected.as_bytes());}
 assert!(clock_view::status(probe::View::Idle).is_none());
}
