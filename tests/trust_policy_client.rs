// SPDX-License-Identifier: GPL-3.0-only
//! Compile the actual held-route client and ABI; no live BOOT or network claim.
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../crates/sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../apps/kaleidoscope/src/trust_probe.rs"]mod probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;
pub const ERR_SERVICE_NOT_READY:u64=9;
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<World>=Mutex::new(World {statuses:Vec::new(),calls:Vec::new(),accepted:Vec::new()});
struct World {statuses:Vec<u64>,calls:Vec<u32>,accepted:Vec<u32>}
fn reset(statuses:&[u64]) {*WORLD.lock().unwrap()=World {statuses:statuses.to_vec(),calls:vec![],accepted:vec![]};}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64) {
    assert_eq!((slot,op),(SLOT_NET,trust_policy::OP));
    let request=trust_policy::Request::decode([a,b,c]).unwrap();
    assert_eq!((a,b,c),(request.seq as u64,0,0));
    let mut w=WORLD.lock().unwrap();w.calls.push(request.seq);
    let status=if w.statuses.is_empty() {0}else{w.statuses.remove(0)};
    if status==0 {assert!(w.accepted.last().is_none_or(|s|request.seq>*s),"accepted query replayed");w.accepted.push(request.seq);}
    (status,0)
}
fn success(seq:u32,profile:u32)->u64 {(0x7Cu64<<56)|((seq as u64)<<32)|profile as u64}
fn failure(seq:u32)->u64 {(0xECu64<<56)|((seq as u64)<<32)|1}
fn ready(p:&mut probe::Client,now:u64) {assert!(p.start());let seq=p.next_sequence()-1;p.advance(now);p.reply(success(seq,1),now+1);assert_eq!(p.view(),probe::View::Ready(1));}
#[test]fn explicit_queries_only_publish_each_correlated_profile_once() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();
    assert_eq!(p.view(),probe::View::Idle);assert_eq!(p.next_sequence(),1);p.advance(0);assert!(WORLD.lock().unwrap().calls.is_empty());
    assert!(p.start());assert!(!p.start());assert_eq!(p.take_profile(),None);p.advance(10);p.advance(11);p.reply(success(1,1),12);
    assert_eq!(p.view(),probe::View::Ready(1));assert_eq!(p.take_profile(),Some(1));assert_eq!(p.take_profile(),None);
    ready(&mut p,20);assert_eq!(p.take_profile(),Some(1));assert_eq!(WORLD.lock().unwrap().accepted,vec![1,2]);
}
#[test]fn every_new_query_discards_unconsumed_profile() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();ready(&mut p,1);
    assert!(p.start());assert_eq!(p.take_profile(),None);assert_eq!(p.view(),probe::View::Pending);p.advance(3);p.reply(failure(2),4);
    assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_profile(),None);
}
#[test]fn only_pre_effect_full_retries_preserve_the_same_sequence() {
    let _serial=SERIAL.lock().unwrap();reset(&[ERR_SERVICE_NOT_READY,ERR_SERVICE_NOT_READY,0]);let mut p=probe::Client::new();
    assert!(p.start());for now in 10..20 {p.advance(now);}p.reply(success(1,1),20);
    assert_eq!(p.take_profile(),Some(1));let w=WORLD.lock().unwrap();assert_eq!(w.calls,vec![1,1,1]);assert_eq!(w.accepted,vec![1]);
}
#[test]fn pre_effect_full_deadline_stops_without_hidden_retry_or_wrap() {
    let _serial=SERIAL.lock().unwrap();reset(&[ERR_SERVICE_NOT_READY,ERR_SERVICE_NOT_READY]);let mut p=probe::Client::new();
    assert!(p.start());p.advance(100);p.advance(2099);p.advance(2100);p.advance(3000);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.next_sequence(),2);assert_eq!(WORLD.lock().unwrap().calls,vec![1,1]);
}
#[test]fn backwards_time_before_admission_stops_without_retry() {
    let _serial=SERIAL.lock().unwrap();reset(&[ERR_SERVICE_NOT_READY]);let mut p=probe::Client::new();
    assert!(p.start());p.advance(100);p.advance(99);p.advance(101);assert_eq!(p.view(),probe::View::Unavailable);assert!(!p.busy());assert_eq!(WORLD.lock().unwrap().calls,vec![1]);
}
#[test]fn rejected_admission_is_not_retried_and_does_not_accept_a_forged_reply() {
    let _serial=SERIAL.lock().unwrap();reset(&[123]);let mut p=probe::Client::new();assert!(p.start());p.advance(1);p.advance(2);p.reply(success(1,1),3);
    assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_profile(),None);assert!(!p.busy());assert_eq!(WORLD.lock().unwrap().calls,vec![1]);
}
#[test]fn reply_before_admission_wrong_tag_and_wrong_sequence_cannot_publish() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();assert!(p.start());p.reply(success(1,1),1);assert_eq!(p.view(),probe::View::Pending);
    p.advance(2);for word in [success(2,1),(0x7Bu64<<56)|(1<<32)|1,(0xEBu64<<56)|(1<<32)|1] {p.reply(word,3);assert!(p.busy());assert_eq!(p.take_profile(),None);}
    p.reply(success(1,1),4);assert_eq!(p.take_profile(),Some(1));
}
#[test]fn cancel_before_admission_consumes_sequence_and_discards_old_profile() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();ready(&mut p,1);assert!(p.start());p.cancel();p.advance(3);p.reply(success(2,1),4);
    assert_eq!(p.take_profile(),None);assert_eq!(p.view(),probe::View::Cancelled);assert!(!p.busy());assert_eq!(p.next_sequence(),3);assert_eq!(WORLD.lock().unwrap().calls,vec![1]);
}
#[test]fn cancel_after_full_stops_retry_without_reusing_sequence() {
    let _serial=SERIAL.lock().unwrap();reset(&[ERR_SERVICE_NOT_READY]);let mut p=probe::Client::new();assert!(p.start());p.advance(1);p.cancel();p.advance(2);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);ready(&mut p,3);assert_eq!(WORLD.lock().unwrap().calls,vec![1,2]);
}
#[test]fn accepted_cancel_retains_correlation_until_late_ack_without_publication() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();assert!(p.start());p.advance(1);p.cancel();assert!(p.busy());assert!(!p.start());assert_eq!(p.view(),probe::View::Uncertain);
    for now in [2,2001,3000] {p.advance(now);}assert_eq!(WORLD.lock().unwrap().calls,vec![1]);p.reply(success(2,1),3001);assert!(p.busy());
    p.reply(success(1,1),3002);assert!(!p.busy());assert_eq!(p.view(),probe::View::Cancelled);assert_eq!(p.take_profile(),None);assert!(p.start());
}
#[test]fn accepted_timeout_never_replays_or_falls_back_after_late_ack() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();ready(&mut p,1);assert!(p.start());p.advance(100);p.advance(2100);p.advance(2101);
    assert!(p.busy());assert!(!p.start());assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_profile(),None);p.reply(success(2,1),2102);
    assert!(!p.busy());assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_profile(),None);assert_eq!(WORLD.lock().unwrap().calls,vec![1,2]);assert!(p.start());
}
#[test]fn reply_enforces_deadline_and_backwards_clock_without_an_advance_turn() {
    let _serial=SERIAL.lock().unwrap();for now in [2100,99] {reset(&[]);let mut p=probe::Client::new();assert!(p.start());p.advance(100);p.reply(success(1,1),now);
        assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_profile(),None);assert!(!p.busy());}
}
#[test]fn unknown_profile_and_service_failures_never_become_authorized_or_reuse_old_data() {
    let _serial=SERIAL.lock().unwrap();for word in [success(2,0),success(2,2),success(2,u32::MAX),failure(2)] {reset(&[]);let mut p=probe::Client::new();ready(&mut p,1);
        assert!(p.start());p.advance(3);p.reply(word,4);assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(p.take_profile(),None);assert!(!p.busy());}
}
#[test]fn close_is_terminal_before_during_and_after_query() {
    let _serial=SERIAL.lock().unwrap();for phase in 0..3 {reset(&[]);let mut p=probe::Client::new();if phase>0 {assert!(p.start());p.advance(1);}if phase==2 {p.reply(success(1,1),2);}
        p.close();p.reply(success(1,1),3);p.advance(4);p.cancel();assert!(!p.start());assert!(!p.busy());assert_eq!(p.take_profile(),None);assert_eq!(p.view(),probe::View::Cancelled);
        assert_eq!(WORLD.lock().unwrap().calls.len(),if phase==0 {0}else{1});}
}
#[test]fn finite_last_sequence_succeeds_then_stops_without_wrap_or_stale_profile() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();p.set_next_sequence_for_test(trust_policy::MAX_SEQ);ready(&mut p,1);
    assert_eq!(p.next_sequence(),trust_policy::MAX_SEQ+1);assert!(!p.start());p.advance(3);assert_eq!(p.take_profile(),None);assert_eq!(p.view(),probe::View::Unavailable);assert_eq!(WORLD.lock().unwrap().calls,vec![trust_policy::MAX_SEQ]);
}
#[test]fn timeout_is_latched_when_time_later_moves_back_inside_the_original_window() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();assert!(p.start());p.advance(100);p.advance(2100);p.reply(success(1,1),101);
    assert_eq!(p.view(),probe::View::Uncertain);assert_eq!(p.take_profile(),None);assert!(!p.busy());assert_eq!(WORLD.lock().unwrap().calls,vec![1]);
}
#[test]fn saturated_deadline_fails_before_effect_and_cannot_wrap_into_freshness() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();assert!(p.start());p.advance(u64::MAX);p.advance(0);
    assert_eq!(p.view(),probe::View::Unavailable);assert!(!p.busy());assert_eq!(p.take_profile(),None);assert!(WORLD.lock().unwrap().calls.is_empty());
}
#[test]fn old_completed_receipt_cannot_complete_the_next_query() {
    let _serial=SERIAL.lock().unwrap();reset(&[]);let mut p=probe::Client::new();ready(&mut p,1);assert!(p.start());p.advance(3);p.reply(success(1,1),4);
    assert!(p.busy());assert_eq!(p.view(),probe::View::Pending);assert_eq!(p.take_profile(),None);p.reply(success(2,1),5);assert_eq!(p.take_profile(),Some(1));
}
