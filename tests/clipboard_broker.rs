// SPDX-License-Identifier: GPL-3.0-only
//! Actual BOOT clipboard ABI and broker; host callbacks model only IPC admission.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path = "../crates/sex-pdx/src/text_encoding.rs"] pub mod text_encoding;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path = "../crates/sex-pdx/src/text_selection.rs"] pub mod text_selection;
#[path = "../crates/sex-pdx/src/clipboard.rs"] pub mod clipboard;
#[path = "../crates/sex-pdx/src/clipboard_client.rs"] pub mod clipboard_client;
mod broker {
    include!("../servers/silk-shell/src/clipboard.rs");
    pub fn clean(b: &Broker) -> (bool, bool, bool) {
        (b.stage.iter().all(|&x| x == 0), b.committed.iter().all(|&x| x == 0), b.read_snapshot.iter().all(|&x| x == 0))
    }
    pub fn stage(b: &Broker) -> &[u8] { &b.stage }
    pub fn committed(b: &Broker) -> &[u8] { &b.committed }
    pub fn snapshot(b: &Broker) -> &[u8] { &b.read_snapshot }
    pub fn packet(b: &Broker, index: usize) -> Option<Packet> { b.slots[index].pending.as_ref().map(|p| p.packet) }
    pub fn counters(b: &mut Broker, ticket: u32, generation: u32) { b.ticket = ticket; b.generation = generation; }
    pub fn near_total_deadline(b: &mut Broker) { b.transaction.as_mut().unwrap().turns = TOTAL_TURNS - 1; }
}
use app_lifecycle::CatalogInfo;
use broker::{Admission, Broker};
use clipboard::*;
use text_selection::Source;
const Q: Subject = Subject { pd: 9, task: 90 };
const B: Subject = Subject { pd: 15, task: 150 };
const Q2: Subject = Subject { pd: 16, task: 160 };
const Q3: Subject = Subject { pd: 18, task: 180 };
const SOURCE: Source = Source { generation: 7, revision: 11 };
fn catalog(s: Subject) -> CatalogInfo {
    CatalogInfo { target_pd: s.pd, task_id: s.task, surface_id: app_catalog::by_target(s.pd).map_or(999, |row| row.surface), coordinator_pd: 3 }
}
fn fresh() -> Broker { let mut b = Broker::new(); for s in [Q, B, Q2, Q3] { b.register(catalog(s)).unwrap(); } b }
fn receive(b: &mut Broker, s: Subject, r: Request, now: u64) -> Result<(), Status> {
    let p = r.wire(); let decoded = Request::decode(p.opcode, p.args).unwrap(); assert_eq!(decoded, r);
    b.receive(s, decoded, true, now)
}
fn admitted(b: &mut Broker, now: u64) -> Vec<(Subject, Response)> {
    let mut out = Vec::new();
    b.poll(now, 4, |p, w| { out.push((p.subject, Response::decode(w.opcode, w.args).unwrap())); Admission::Accepted }); out
}
fn intent(b: &mut Broker, s: Subject, a: Action, now: u64) -> u32 {
    let t = b.issue(s, a, true, now).unwrap();
    assert!(admitted(b, now).contains(&(s, Response::Intent { ticket: t, action: a }))); t
}
fn begin(b: &mut Broker, s: Subject, body: &[u8], now: u64) -> u32 {
    let t = intent(b, s, Action::Copy, now); receive(b, s, Request::Begin { ticket: t, len: body.len(), source: SOURCE }, now).unwrap();
    assert!(admitted(b, now).iter().any(|(_, r)| matches!(r, Response::Meta { phase: Phase::Begin, status: Status::Ok, len, generation: 0, hash: 0, .. } if *len == body.len()))); t
}
fn write(b: &mut Broker, s: Subject, ticket: u32, body: &[u8], now: u64) {
    for (i, chunk) in body.chunks(16).enumerate() { let mut bytes = [0; 16]; bytes[..chunk.len()].copy_from_slice(chunk);
        receive(b, s, Request::Write { ticket, offset: i * 16, count: chunk.len(), bytes }, now).unwrap(); }
}
fn publish(b: &mut Broker, s: Subject, body: &[u8], now: u64, ack: bool) -> u32 {
    let t = begin(b, s, body, now); write(b, s, t, body, now);
    receive(b, s, Request::Publish { ticket: t, len: body.len(), hash: hash(body) }, now).unwrap();
    if ack { admitted(b, now); } t
}
fn read(b: &mut Broker, s: Subject, now: u64) -> u32 {
    let t = intent(b, s, Action::Paste, now); receive(b, s, Request::Read { ticket: t }, now).unwrap(); admitted(b, now); t
}
fn read_all(b: &mut Broker, s: Subject, t: u32, len: usize, now: u64) -> Vec<u8> {
    let mut body = Vec::new();
    for offset in (0..len).step_by(16) {
        receive(b, s, Request::Get { ticket: t, offset }, now).unwrap();
        for (who, r) in admitted(b, now) { assert_eq!(who, s); match r { Response::Data { ticket, offset: at, count, bytes } => {
            assert_eq!((ticket, at, count), (t, offset, usize::min(16, len - offset))); assert!(bytes[count..].iter().all(|&x| x == 0)); body.extend_from_slice(&bytes[..count]);
        }, _ => panic!("not data") } }
    } body
}
fn status(r: Response, phase: Phase, expected: Status) {
    assert!(matches!(r, Response::Meta { phase: p, status: s, .. } if p == phase && s == expected), "{r:?}");
}

#[test] fn exact_immutable_catalog_rights_and_terminal_fence() {
    let mut b = fresh(); assert_eq!(b.register(catalog(Q)), Ok(Participant::from_catalog(catalog(Q)).unwrap()));
    let mut c = catalog(Q); c.task_id += 1; assert_eq!(b.register(c), Err(Status::Denied)); c = catalog(Q); c.surface_id += 1; assert_eq!(b.register(c), Err(Status::Denied));
    for s in [Subject { pd: 99, task: 9 }, Subject { pd: 9, task: 9 }] { assert_eq!(b.issue(s, Action::Copy, true, 0), Err(Status::Denied)); }
    assert_eq!(b.issue(B, Action::Cut, true, 0), Err(Status::Denied)); assert_eq!(b.issue(B, Action::Paste, true, 0), Err(Status::Denied));
    assert_eq!(b.issue(Q, Action::Copy, false, 0), Err(Status::Denied)); assert!(!b.pending());
    assert!(b.retire(Q)); b.register(catalog(Q)).unwrap(); assert_eq!(b.issue(Q, Action::Copy, true, 0), Err(Status::Denied));
    assert_eq!(b.receive(Q, Request::Cancel { ticket: 1 }, true, 0), Err(Status::Denied)); assert!(!b.retire(Subject { pd: 9, task: 9 }));
}

#[test] fn intent_and_begin_handshakes_require_actual_admission_full_is_identical() {
    let mut b = fresh(); let t = b.issue(Q, Action::Cut, true, 1).unwrap(); let before = broker::packet(&b, 0).unwrap();
    for _ in 0..3 { assert_eq!(b.poll(1, 4, |_, p| { assert_eq!(p, before); Admission::Full }), 1); }
    assert_eq!(receive(&mut b, Q, Request::Begin { ticket: t, len: 3, source: SOURCE }, 1), Err(Status::Busy));
    let victim = broker::packet(&b, 0);
    assert_eq!(b.receive(Subject { pd: 9, task: 9 }, Request::Cancel { ticket: t }, true, 999999), Err(Status::Denied));
    assert_eq!(b.receive(B, Request::Cancel { ticket: t }, true, 999999), Err(Status::Denied)); assert_eq!(broker::packet(&b, 0), victim);
    admitted(&mut b, 1); receive(&mut b, Q, Request::Begin { ticket: t, len: 3, source: SOURCE }, 1).unwrap();
    let packet = broker::packet(&b, 0).unwrap(); b.poll(1, 4, |_, p| { assert_eq!(p, packet); Admission::Full });
    assert_eq!(receive(&mut b, Q, Request::Write { ticket: t, offset: 0, count: 3, bytes: *b"abc\0\0\0\0\0\0\0\0\0\0\0\0\0" }, 1), Err(Status::Busy));
    admitted(&mut b, 1); write(&mut b, Q, t, b"abc", 1); assert_eq!(&broker::stage(&b)[..3], b"abc");
}

#[test] fn full_12k_publish_and_read_actual_abi_no_data_ack_or_extra_storage() {
    let mut b = fresh(); let body: Vec<u8> = (0..MAX_BYTES).map(|i| if i % 73 == 0 { b'\n' } else { b' '+(i%95)as u8 }).collect();
    publish(&mut b, B, &body, 1, true); let p = b.publication().unwrap(); assert_eq!((p.owner,p.source,p.len,p.generation,p.hash),(B,SOURCE,MAX_BYTES,1,hash(&body)));
    assert_eq!(broker::clean(&b),(true,false,true)); let t = read(&mut b,Q,1); assert_eq!(read_all(&mut b,Q,t,body.len(),1),body);
    receive(&mut b,Q,Request::Finish { ticket:t,generation:p.generation,hash:p.hash },1).unwrap(); assert!(b.busy());
    let receipt=admitted(&mut b,1); status(receipt[0].1,Phase::Finish,Status::Ok); assert!(!b.busy()); assert_eq!(broker::clean(&b),(true,false,true));
    assert!(core::mem::size_of::<Broker>()<3*MAX_BYTES+4096);
}

#[test] fn full_publish_receipt_is_immutable_and_does_not_block_other_participant() {
    let mut b=fresh(); let t=publish(&mut b,Q,b"first",1,false); let publication=b.publication().unwrap(); let packet=broker::packet(&b,0).unwrap();
    b.poll(999999,4,|_,p| {assert_eq!(p,packet);Admission::Full}); assert_eq!(b.publication(),Some(publication)); assert_eq!(broker::packet(&b,0),Some(packet));
    assert_eq!(b.issue(Q,Action::Copy,true,999999),Err(Status::Busy));
    let other=b.issue(Q2,Action::Paste,true,999999).unwrap(); assert!(other>t); admitted(&mut b,999999);
    assert_eq!(receive(&mut b,Q,Request::Publish {ticket:t,len:5,hash:hash(b"first")},999999),Err(Status::Denied)); assert_eq!(b.publication(),Some(publication));
    receive(&mut b,Q,Request::Query {ticket:t},999999).unwrap(); assert_eq!(broker::packet(&b,0),Some(packet)); admitted(&mut b,999999); assert_eq!(b.publication(),Some(publication));
}

#[test] fn malformed_order_tail_and_hash_poison_whole_stage_without_replacing_publication() {
    for bad in 0..5 {
        let mut b=fresh(); publish(&mut b,B,b"old",0,true); let old=b.publication(); let body=b"abcdefghijklmnopq"; let t=begin(&mut b,Q,body,1); write(&mut b,Q,t,&body[..16],1);
        let mut bytes=[0;16]; bytes[0]=b'q'; let (offset,count)=match bad {0=>(32,1),1=>(16,2),_ =>(16,1)};
        if bad==2 {bytes[1]=1;} if bad==3 {bytes[0]=0xff;}
        if bad<3 {assert_eq!(b.receive(Q,Request::Write{ticket:t,offset,count,bytes},true,1),Err(Status::Invalid)); assert!(broker::clean(&b).0);}
        else {receive(&mut b,Q,Request::Write{ticket:t,offset:16,count:1,bytes},1).unwrap();}
        assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:17,hash:0},1),Err(Status::Invalid)); status(admitted(&mut b,1)[0].1,Phase::Publish,Status::Invalid);
        assert_eq!(b.publication(),old);assert!(broker::clean(&b).0);assert_eq!(&broker::committed(&b)[..3],b"old");
    }
}

#[test] fn duplicate_publish_query_and_cancel_never_reapply_or_rollback_committed_bytes() {
    let mut b=fresh();let t=publish(&mut b,Q,b"committed",1,false);let p=b.publication();
    let pending=broker::packet(&b,0);receive(&mut b,Q,Request::Query{ticket:t},1).unwrap();assert_eq!(broker::packet(&b,0),pending);
    assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:9,hash:hash(b"committed")},1),Err(Status::Denied));
    receive(&mut b,Q,Request::Cancel{ticket:t},1).unwrap(); status(admitted(&mut b,1)[0].1,Phase::Cancel,Status::Ok);assert_eq!(b.publication(),p);
    receive(&mut b,Q,Request::Query{ticket:t},1).unwrap();status(admitted(&mut b,1)[0].1,Phase::Cancel,Status::Ok);assert_eq!(b.publication(),p);
}

#[test] fn read_meta_data_and_finish_full_only_advance_on_accepted_and_cancel_scrubs_snapshot() {
    let mut b=fresh();publish(&mut b,B,b"abcdefghijklmnopq",1,true);let t=intent(&mut b,Q,Action::Paste,1);receive(&mut b,Q,Request::Read{ticket:t},1).unwrap();
    let m=broker::packet(&b,0).unwrap(); b.poll(1,4,|_,p|{assert_eq!(p,m);Admission::Full}); assert_eq!(receive(&mut b,Q,Request::Get{ticket:t,offset:0},1),Err(Status::Busy));admitted(&mut b,1);
    receive(&mut b,Q,Request::Get{ticket:t,offset:0},1).unwrap();let d=broker::packet(&b,0).unwrap();b.poll(1,4,|_,p|{assert_eq!(p,d);Admission::Full});
    assert_eq!(receive(&mut b,Q,Request::Get{ticket:t,offset:16},1),Err(Status::Busy));admitted(&mut b,1);
    assert_eq!(receive(&mut b,Q,Request::Get{ticket:t,offset:0},1),Err(Status::Invalid));receive(&mut b,Q,Request::Get{ticket:t,offset:16},1).unwrap();
    receive(&mut b,Q,Request::Cancel{ticket:t},1).unwrap();assert_eq!(broker::clean(&b),(true,false,true));status(admitted(&mut b,1)[0].1,Phase::Cancel,Status::Ok);assert!(!b.busy());
}

#[test] fn producer_retirement_before_finish_admission_invalidates_every_unfinished_read_edge() {
    for edge in 0..4 {
        let mut b=fresh();publish(&mut b,B,b"private",1,true);let p=b.publication().unwrap();let t=intent(&mut b,Q,Action::Paste,1);receive(&mut b,Q,Request::Read{ticket:t},1).unwrap();
        if edge>=1 {admitted(&mut b,1);receive(&mut b,Q,Request::Get{ticket:t,offset:0},1).unwrap();}
        if edge>=2 {admitted(&mut b,1);}
        if edge==3 {receive(&mut b,Q,Request::Finish{ticket:t,generation:p.generation,hash:p.hash},1).unwrap();let receipt=broker::packet(&b,0).unwrap();b.poll(1,4,|_,w|{assert_eq!(w,receipt);Admission::Full});}
        assert!(b.retire(B));assert_eq!(broker::clean(&b),(true,true,true));assert!(b.publication().is_none());status(admitted(&mut b,1)[0].1,Phase::Finish,Status::Invalidated);assert!(!b.busy());
    }
}

#[test] fn accepted_finish_hands_off_and_source_close_cannot_revoke_completed_transfer() {
    let mut b=fresh();publish(&mut b,B,b"handed off",1,true);let p=b.publication().unwrap();let t=read(&mut b,Q,1);let received=read_all(&mut b,Q,t,p.len,1);
    receive(&mut b,Q,Request::Finish{ticket:t,generation:p.generation,hash:p.hash},1).unwrap();let receipt=admitted(&mut b,1);status(receipt[0].1,Phase::Finish,Status::Ok);
    b.retire(B);assert!(!b.pending());assert_eq!(received,b"handed off");assert_eq!(broker::clean(&b),(true,true,true));
    receive(&mut b,Q,Request::Query{ticket:t},1).unwrap();status(admitted(&mut b,1)[0].1,Phase::Finish,Status::Ok);
}

#[test] fn closing_old_producer_preserves_new_version_and_consumer_close_only_scrubs_its_snapshot() {
    let mut b=fresh();publish(&mut b,B,b"old long publication",1,true);publish(&mut b,Q2,b"new",1,true);let p=b.publication().unwrap();assert_eq!(p.generation,2);assert!(broker::committed(&b)[3..].iter().all(|&x|x==0));
    let t=read(&mut b,Q,1);receive(&mut b,Q,Request::Get{ticket:t,offset:0},1).unwrap();b.retire(B);assert_eq!(b.publication(),Some(p));assert_eq!(&broker::snapshot(&b)[..3],b"new");
    b.retire(Q);assert_eq!(b.publication(),Some(p));assert_eq!(broker::clean(&b),(true,false,true));assert!(!b.pending());
}

#[test] fn source_version_is_exact_and_finish_requires_all_data_exact_generation_and_hash() {
    let mut b=fresh();publish(&mut b,Q2,b"data",1,true);let p=b.publication().unwrap();assert_eq!(p.source,SOURCE);let t=read(&mut b,Q,1);
    for (g,h) in [(p.generation,p.hash),(p.generation+1,p.hash),(p.generation,p.hash^1)] {assert_eq!(receive(&mut b,Q,Request::Finish{ticket:t,generation:g,hash:h},1),Err(Status::Invalid));}
    read_all(&mut b,Q,t,4,1);assert_eq!(receive(&mut b,Q,Request::Finish{ticket:t,generation:p.generation+1,hash:p.hash},1),Err(Status::Invalid));
    receive(&mut b,Q,Request::Finish{ticket:t,generation:p.generation,hash:p.hash},1).unwrap();let packet=broker::packet(&b,0).unwrap();for _ in 0..3{b.poll(1,4,|_,p|{assert_eq!(p,packet);Admission::Full});}assert!(b.busy());admitted(&mut b,1);assert!(!b.busy());
}

#[test] fn frozen_backward_and_tick_deadlines_wipe_unpublished_data_and_query_is_readonly() {
    for mode in 0..3 {
        let mut b=fresh();publish(&mut b,B,b"committed",10,true);let t=begin(&mut b,Q,b"unpublished",10);write(&mut b,Q,t,b"unpublished",10);
        match mode {0=>{for _ in 0..broker::STALL_TURNS {b.poll(10,0,|_,_|panic!());}},1=>{b.poll(9,0,|_,_|panic!());},_=>{b.poll(10+broker::STALL_TICKS,0,|_,_|panic!());}}
        assert_eq!(broker::clean(&b),(true,false,true));assert!(!b.busy());let packet=broker::packet(&b,0);receive(&mut b,Q,Request::Query{ticket:t},10).unwrap();assert_eq!(broker::packet(&b,0),packet);status(admitted(&mut b,10)[0].1,Phase::Publish,Status::Uncertain);
        assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:11,hash:hash(b"unpublished")},10),Err(Status::Denied));assert_eq!(&broker::committed(&b)[..9],b"committed");
    }
}

#[test] fn cancel_pending_intent_or_upload_quiesces_without_publish_and_is_replay_safe() {
    for upload in [false,true] {
        let mut b=fresh();let t=if upload{let t=begin(&mut b,Q,b"some text",0);write(&mut b,Q,t,b"some text",0);t}else{b.issue(Q,Action::Copy,true,0).unwrap()};
        receive(&mut b,Q,Request::Cancel{ticket:t},0).unwrap();let packet=broker::packet(&b,0);b.poll(0,4,|_,p|{assert_eq!(Some(p),packet);Admission::Full});assert_eq!(broker::clean(&b),(true,true,true));assert!(!b.busy());
        receive(&mut b,Q,Request::Cancel{ticket:t},0).unwrap();assert_eq!(broker::packet(&b,0),packet);status(admitted(&mut b,0)[0].1,Phase::Cancel,Status::Ok);
    }
}

#[test] fn query_replay_does_not_extend_stall_and_real_write_does_not_reset_total_deadline() {
    let mut b=fresh();let t=begin(&mut b,Q,b"pending",1);assert!(b.pending());
    assert_eq!(b.issue(Q2,Action::Copy,true,1),Err(Status::Busy));
    for _ in 0..broker::STALL_TURNS {receive(&mut b,Q,Request::Query{ticket:t},1).unwrap();admitted(&mut b,1);}
    assert!(!b.busy());receive(&mut b,Q,Request::Query{ticket:t},1).unwrap();status(admitted(&mut b,1)[0].1,Phase::Publish,Status::Uncertain);assert_eq!(broker::clean(&b),(true,true,true));
    let mut b=fresh();let t=begin(&mut b,Q,b"new",1);broker::near_total_deadline(&mut b);write(&mut b,Q,t,b"new",1);b.poll(1,0,|_,_|panic!());
    status(admitted(&mut b,1)[0].1,Phase::Publish,Status::Uncertain);assert_eq!(broker::clean(&b),(true,true,true));
}

#[test] fn exhaustion_never_wraps_ticket_or_generation_and_preserves_current_publication() {
    let mut b=fresh();broker::counters(&mut b,u32::MAX,0);assert_eq!(b.issue(Q,Action::Copy,true,0),Err(Status::Exhausted));assert!(!b.pending());
    let mut b=fresh();publish(&mut b,B,b"keep",0,true);let p=b.publication();let t=begin(&mut b,Q,b"replace",0);write(&mut b,Q,t,b"replace",0);broker::counters(&mut b,t,u32::MAX);
    assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:7,hash:hash(b"replace")},0),Err(Status::Exhausted));assert_eq!(b.publication(),p);assert!(broker::clean(&b).0);status(admitted(&mut b,0)[0].1,Phase::Publish,Status::Exhausted);
}

#[test] fn round_robin_budget_and_route_rejection_scrub_exact_owner_before_fence() {
    let mut b=fresh();publish(&mut b,B,b"source",0,false);let t=b.issue(Q,Action::Paste,true,0).unwrap();let mut seen=Vec::new();
    assert_eq!(b.poll(0,2,|p,_|{seen.push(p.subject);Admission::Full}),2);assert_eq!(seen,[Q,B]);
    admitted(&mut b,0);receive(&mut b,Q,Request::Read{ticket:t},0).unwrap();admitted(&mut b,0);
    // Requeue scalar producer receipt so rejection races an active snapshot.
    receive(&mut b,B,Request::Query{ticket:1},0).unwrap();b.poll(0,1,|p,_|{assert_eq!(p.subject,B);Admission::Rejected(7)});
    assert_eq!(broker::clean(&b),(true,true,true));assert_eq!(b.issue(B,Action::Copy,true,0),Err(Status::Denied));status(admitted(&mut b,0)[0].1,Phase::Finish,Status::Invalidated);
}

fn participant(s: Subject) -> Participant { Participant::from_catalog(catalog(s)).unwrap() }
fn client_step(b: &mut Broker, s: Subject, c: &mut clipboard_client::Client, now: u64) -> usize {
    c.poll(now, |p| {receive(b,s,Request::decode(p.opcode,p.args).unwrap(),now).unwrap();clipboard_client::Admission::Accepted})
}
fn broker_step(b: &mut Broker, s: Subject, c: &mut clipboard_client::Client, now: u64, full_once: &mut Vec<Packet>) {
    b.poll(now,4,|p,w| {
        assert_eq!(p,participant(s));
        if !full_once.contains(&w) {full_once.push(w);return Admission::Full}
        assert!(c.receive(SHELL,w.opcode,w.args,now));Admission::Accepted
    });
}

#[test] fn actual_client_and_broker_full_12k_copy_paste_with_full_at_each_response_phase() {
    use clipboard_client::{Client,Event,View};
    let mut b=fresh();let mut producer=Client::new();let mut consumer=Client::new();assert!(producer.bind(participant(B)));assert!(consumer.bind(participant(Q)));
    let body:Vec<u8>=(0..MAX_BYTES).map(|i|if i%97==0{b'\n'}else{b'A'+(i%26)as u8}).collect();let mut full=Vec::new();
    let copy=b.issue(B,Action::Copy,true,1).unwrap();broker_step(&mut b,B,&mut producer,1,&mut full);assert!(producer.take_event().is_none());broker_step(&mut b,B,&mut producer,1,&mut full);
    assert_eq!(producer.take_event(),Some(Event::Export{ticket:copy,cut:false}));assert!(producer.publish(copy,SOURCE,&body,1));
    for _ in 0..2000 {assert!(client_step(&mut b,B,&mut producer,1)<=4);broker_step(&mut b,B,&mut producer,1,&mut full);if producer.view()==View::Copied{break}}
    assert_eq!(producer.view(),View::Copied);assert!(producer.storage_erased());assert!(producer.packet_erased());let p=b.publication().unwrap();assert_eq!((p.owner,p.len,p.hash),(B,MAX_BYTES,hash(&body)));
    assert_eq!(producer.take_event(),Some(Event::Published{ticket:copy,cut:false,len:MAX_BYTES,generation:p.generation,hash:p.hash}));assert!(!b.pending());
    let paste=b.issue(Q,Action::Paste,true,1).unwrap();broker_step(&mut b,Q,&mut consumer,1,&mut full);broker_step(&mut b,Q,&mut consumer,1,&mut full);assert_eq!(consumer.take_event(),Some(Event::Paste{ticket:paste}));assert!(consumer.read(paste,1));
    for _ in 0..3000 {assert!(consumer.bytes().is_none());assert!(client_step(&mut b,Q,&mut consumer,1)<=4);broker_step(&mut b,Q,&mut consumer,1,&mut full);if consumer.view()==View::Ready{break}}
    assert_eq!(consumer.bytes(),Some(body.as_slice()));assert_eq!(consumer.take_event(),Some(Event::Ready{ticket:paste,len:MAX_BYTES,generation:p.generation,hash:p.hash}));assert_eq!(broker::clean(&b),(true,false,true));assert!(!b.pending());
    b.retire(B);assert_eq!(consumer.bytes(),Some(body.as_slice()));assert!(consumer.finish_read());assert!(consumer.storage_erased());assert!(consumer.packet_erased());assert_eq!(broker::clean(&b),(true,true,true));
}

#[test] fn actual_client_cancel_after_admitted_write_quiesces_broker_and_erases_both_scratch_buffers() {
    use clipboard_client::{Client,Event,View};
    let mut b=fresh();let mut c=Client::new();assert!(c.bind(participant(Q)));let mut full=Vec::new();let t=b.issue(Q,Action::Cut,true,1).unwrap();broker_step(&mut b,Q,&mut c,1,&mut full);broker_step(&mut b,Q,&mut c,1,&mut full);
    assert_eq!(c.take_event(),Some(Event::Export{ticket:t,cut:true}));assert!(c.publish(t,SOURCE,&[b'x';512],1));client_step(&mut b,Q,&mut c,1);broker_step(&mut b,Q,&mut c,1,&mut full);broker_step(&mut b,Q,&mut c,1,&mut full);assert!(!b.outbox_pending());assert!(b.pending());
    assert_eq!(client_step(&mut b,Q,&mut c,1),4);assert!(!broker::clean(&b).0);c.cancel(1);assert!(c.storage_erased());client_step(&mut b,Q,&mut c,1);assert_eq!(broker::clean(&b),(true,true,true));assert!(!b.busy());
    broker_step(&mut b,Q,&mut c,1,&mut full);assert_ne!(c.view(),View::Cancelled);broker_step(&mut b,Q,&mut c,1,&mut full);assert_eq!(c.view(),View::Cancelled);assert!(c.packet_erased());assert!(!b.pending());assert!(b.publication().is_none());
}

#[test] fn utf8_scalars_cross_every_16_byte_boundary_without_changing_exact_publication_or_read(){
    for scalar in ["é","€","𐍈"] {for split in 1..scalar.len(){
        let mut body=vec![b'a';MAX_BYTES];for boundary in (16..MAX_BYTES).step_by(16){let start=boundary-split;body[start..start+scalar.len()].copy_from_slice(scalar.as_bytes());}
        assert!(text(&body));assert!(!text(&body[..16]));let mut b=fresh();publish(&mut b,Q,&body,1,true);let p=b.publication().unwrap();
        assert_eq!((p.owner,p.source,p.len,p.hash),(Q,SOURCE,MAX_BYTES,hash(&body)));assert_eq!(&broker::committed(&b)[..MAX_BYTES],body);
        let t=read(&mut b,Q2,1);assert_eq!(read_all(&mut b,Q2,t,MAX_BYTES,1),body);
        receive(&mut b,Q2,Request::Finish{ticket:t,generation:p.generation,hash:p.hash},1).unwrap();status(admitted(&mut b,1)[0].1,Phase::Finish,Status::Ok);assert!(broker::clean(&b).0);assert!(broker::clean(&b).2);
    }}
}

#[test] fn opaque_malformed_whole_uploads_fail_publish_and_preserve_prior_clipboard(){
    let mut bodies=vec![vec![0xff],vec![0xc0,0xaf],vec![0xe2,0x82],vec![0xed,0xa0,0x80],vec![0xf4,0x90,0x80,0x80]];
    for scalar in (0..=159).filter(|c|(*c<32&&*c!=9&&*c!=10)||(127..=159).contains(c)){let mut bytes=[0;4];bodies.push(char::from_u32(scalar).unwrap().encode_utf8(&mut bytes).as_bytes().to_vec());}
    for body in bodies{
        let mut b=fresh();publish(&mut b,B,"prior €".as_bytes(),0,true);let old=b.publication();let t=intent(&mut b,Q,Action::Cut,1);
        receive(&mut b,Q,Request::Begin{ticket:t,len:body.len(),source:SOURCE},1).unwrap();admitted(&mut b,1);write(&mut b,Q,t,&body,1);
        assert_eq!(b.publication(),old);assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:body.len(),hash:hash(&body)},1),Err(Status::Invalid));
        status(admitted(&mut b,1)[0].1,Phase::Publish,Status::Invalid);assert_eq!(b.publication(),old);assert_eq!(&broker::committed(&b)[.."prior €".len()],"prior €".as_bytes());assert!(broker::clean(&b).0);
    }
    let mut b=fresh();publish(&mut b,B,b"prior",0,true);let old=b.publication();let body="012345678901234€".as_bytes();let t=begin(&mut b,Q,body,1);write(&mut b,Q,t,&body[..16],1);
    assert_eq!(receive(&mut b,Q,Request::Publish{ticket:t,len:body.len(),hash:hash(body)},1),Err(Status::Invalid));assert_eq!(b.publication(),old);assert!(broker::clean(&b).0);
}


#[test] fn every_catalog_document_subject_registers_without_replacing_held_task_or_retirement() {
    let mut b=Broker::new();
    for &target in &app_catalog::DOCUMENT_TARGETS {
        // Deliberately use a different task namespace: held catalog task wins.
        let subject=Subject{pd:target,task:target+1000};
        let c=catalog(subject);let p=b.register(c).unwrap();
        assert_eq!(p.subject,subject);
        assert_eq!(p.route,app_catalog::by_target(target).unwrap().close_slot);
        let mut wrong=c;wrong.task_id+=1;assert_eq!(b.register(wrong),Err(Status::Denied));
        wrong=c;wrong.surface_id+=10000;assert_eq!(b.register(wrong),Err(Status::Denied));
        assert!(b.retire(subject));assert_eq!(b.register(c),Ok(p));
        assert_eq!(b.issue(subject,Action::Copy,true,1),Err(Status::Denied));
    }
    for row in app_catalog::spindle_views(){assert_eq!(b.register(catalog(Subject{pd:row.target,task:row.target})),Err(Status::Denied));}
}

#[test] fn eighth_quil_exact_held_subject_receives_browser_copy_and_owns_its_publication() {
    let newest=Subject{pd:23,task:2300};
    assert_eq!(app_catalog::by_target(newest.pd).unwrap().ordinal,7);
    let mut b=fresh();let p=b.register(catalog(newest)).unwrap();
    assert_eq!((p.subject,p.surface,p.route,p.can_read),(newest,312,127,true));
    assert_eq!(b.issue(Subject{pd:23,task:23},Action::Paste,true,1),Err(Status::Denied));
    assert_eq!(b.issue(B,Action::Paste,true,1),Err(Status::Denied));
    assert_eq!(b.issue(B,Action::Cut,true,1),Err(Status::Denied));
    publish(&mut b,B,"exact browser bytes €".as_bytes(),1,true);let publication=b.publication().unwrap();
    let ticket=read(&mut b,newest,1);
    assert_eq!(read_all(&mut b,newest,ticket,publication.len,1),"exact browser bytes €".as_bytes());
    receive(&mut b,newest,Request::Finish{ticket,generation:publication.generation,hash:publication.hash},1).unwrap();
    status(admitted(&mut b,1)[0].1,Phase::Finish,Status::Ok);
    publish(&mut b,newest,b"successor owns these bytes",1,true);let current=b.publication();
    assert!(b.retire(Q));assert!(b.retire(B));assert_eq!(b.publication(),current);
    assert!(b.retire(newest));assert!(b.publication().is_none());assert_eq!(broker::clean(&b),(true,true,true));
    assert_eq!(b.register(catalog(newest)),Ok(p));assert_eq!(b.issue(newest,Action::Copy,true,1),Err(Status::Denied));
}

#[test] fn all_catalog_slots_get_fair_bounded_delivery_and_keep_exact_full_receipts() {
    let mut b=Broker::new();let subjects:Vec<_>=app_catalog::DOCUMENT_TARGETS.iter().map(|&pd|Subject{pd,task:pd+1000}).collect();
    let mut pending=Vec::new();
    for &subject in &subjects {
        b.register(catalog(subject)).unwrap();let ticket=b.issue(subject,Action::Copy,true,1).unwrap();
        receive(&mut b,subject,Request::Cancel{ticket},1).unwrap();
        let index=app_catalog::DOCUMENT_TARGETS.iter().position(|&pd|pd==subject.pd).unwrap();
        pending.push(broker::packet(&b,index).unwrap());
    }
    let mut seen=Vec::new();
    for _ in 0..subjects.len().div_ceil(4) {
        assert!(b.poll(1,99,|p,packet|{
            let index=subjects.iter().position(|s|*s==p.subject).unwrap();assert_eq!(packet,pending[index]);
            seen.push(p.subject);Admission::Full
        })<=4);
    }
    for (index,subject) in subjects.iter().enumerate(){assert!(seen.contains(subject));assert_eq!(broker::packet(&b,index),Some(pending[index]));}
    let mut delivered=Vec::new();
    while b.pending(){let attempts=b.poll(1,99,|p,packet|{
        let index=subjects.iter().position(|s|*s==p.subject).unwrap();assert_eq!(packet,pending[index]);
        assert!(!delivered.contains(&p.subject));delivered.push(p.subject);Admission::Accepted
    });assert!((1..=4).contains(&attempts));}
    assert_eq!(delivered.len(),subjects.len());
}
