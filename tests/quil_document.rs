#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]
macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
pub const ERR_SERVICE_NOT_READY: u64 = 2;
#[path = "../servers/quil/src/document_exchange.rs"]
mod client;
#[path = "../crates/sex-pdx/src/document.rs"]
pub mod document;
#[path = "../servers/sexfiles/src/document.rs"]
mod service;
use client::{Client, Outcome};
use document as w;
use service::{Backend, Service};
struct Fake {
    generation: u32,
    loads: usize,
    fail: bool,
    saved: Vec<Vec<u8>>,
}
impl Backend for Fake {
    fn save(&mut self, caller: u32, _id: u8, bytes: &[u8]) -> Result<(), u32> {
        assert_eq!(caller, 9);
        self.saved.push(bytes.to_vec());
        Ok(())
    }
    fn create(&mut self, _name: [u8; 16]) -> Result<u8, u32> {
        Ok(5)
    }
    fn identity(&mut self, id: u8) -> Result<u32, u32> {
        if id == 4 {
            Err(w::STALE)
        } else {
            Ok(self.generation)
        }
    }
    fn name(&mut self, _: u8) -> Result<[u8; 16], u32> {
        Ok(*b"report\0\0\0\0\0\0\0\0\0\0")
    }
    fn load(&mut self, _: u8, out: &mut [u8; w::MAX]) -> Result<usize, u32> {
        self.loads += 1;
        out[..5].copy_from_slice(b"hello");
        if self.fail {
            Err(w::IO)
        } else {
            Ok(5)
        }
    }
}
fn fake() -> Fake {
    Fake {
        generation: 7,
        loads: 0,
        fail: false,
        saved: Vec::new(),
    }
}
fn rpc(s: &mut Service, b: &mut Fake, pd: u32, r: w::Request) -> u64 {
    if let Some(v) = s.handle(pd, true, r, b) {
        return v;
    }
    let mut v = None;
    s.progress(b);
    s.poll(|p, x| {
        if p == pd {
            v = Some(x);
            0
        } else {
            1
        }
    });
    v.unwrap()
}
fn drive(c: &mut Client, s: &mut Service, b: &mut Fake) -> client::Event {
    for _ in 0..100 {
        let mut requests = vec![];
        if let Some(e) = c.step(|x| { requests.push(x); Ok(()) }) { return e; }
        for r in requests {
            let v = rpc(s, b, 9, r);
            if let Some(e) = c.receive(11, v) { return e; }
        }
    }
    panic!()
}
#[test]
fn exact_sender_sequence_and_atomic_snapshot() {
    let mut c = Client::new();
    assert!(c.load(3, 42));
    let mut r = None;
    c.step(|x| {
        r = Some(x);
        Ok(())
    });
    assert!(c.receive(3, w::reply(1, Ok(7))).is_none());
    assert!(c.receive(11, w::reply(2, Ok(7))).is_none());
    assert!(c.receive(11, 0).is_none());
    let mut s = Service::new();
    let mut b = fake();
    let v = rpc(&mut s, &mut b, 9, r.unwrap());
    c.receive(11, v);
    let e = drive(&mut c, &mut s, &mut b);
    assert_eq!(e.outcome, Outcome::Loaded);
    assert_eq!(e.revision, 42);
    assert_eq!(&c.bytes[..c.len], b"hello");
    assert_eq!(b.loads, 1);
}
#[test]
fn full_retains_request_and_frozen_clock_has_deadline() {
    let mut c = Client::new();
    c.load(3, 1);
    let mut old = None;
    c.step(|r| {
        old = Some(r);
        Err(ERR_SERVICE_NOT_READY)
    });
    c.step(|r| {
        assert_eq!(Some(r), old);
        Ok(())
    });
    let mut event = None;
    for _ in 0..1_048_576 {
        event = c.step(|_| panic!());
        if event.is_some() {
            break;
        }
    }
    assert_eq!(event.unwrap().outcome, Outcome::Uncertain);
    assert!(!c.load(3, 2));
    assert!(c.receive(11, w::reply(1, Ok(7))).is_none());
    c.close();
    assert!(!c.load(3, 3));
}
#[test]
fn partial_backend_failure_cannot_publish() {
    let mut c = Client::new();
    c.load(3, 0);
    let mut b = fake();
    b.fail = true;
    let e = drive(&mut c, &mut Service::new(), &mut b);
    assert_eq!(e.outcome, Outcome::Failed);
    assert_eq!(c.len, 0);
    assert!(c.bytes.iter().all(|b| *b == 0));
}
#[test]
fn foreign_dead_and_recycled_bindings_reject_before_effects() {
    let mut s = Service::new();
    let mut b = fake();
    let mut r = w::Request {
        seq: 1,
        kind: w::LOAD,
        object: 3,
        offset: 0,
        a: 7,
        b: 0,
    };
    assert_eq!(
        w::decode_reply(rpc(&mut s, &mut b, 12, r), 1),
        Some(Err(w::DENIED))
    );
    assert_eq!(b.loads, 0);
    b.generation = 8;
    assert_eq!(
        w::decode_reply(rpc(&mut s, &mut b, 9, r), 1),
        Some(Err(w::STALE))
    );
    assert_eq!(b.loads, 0);
    s.retire(9, &mut b);
    r.seq = 2;
    r.a = 8;
    assert_eq!(
        w::decode_reply(rpc(&mut s, &mut b, 9, r), 2),
        Some(Err(w::DENIED))
    );
    assert_eq!(b.loads, 0);
}
#[test]
fn duplicate_effect_once_pending_receipts_survive_full() {
    let mut s = Service::new();
    let mut b = fake();
    let r = w::Request {
        seq: 1,
        kind: w::LOAD,
        object: 3,
        offset: 0,
        a: 7,
        b: 0,
    };
    s.handle(9, true, r, &mut b);
    s.poll(|_, _| 1);
    assert!(s.pending());
    s.handle(9, true, r, &mut b);
    assert_eq!(b.loads, 1);
    s.poll(|_, _| 0);
    assert_eq!(w::decode_reply(rpc(&mut s, &mut b, 9, r), 1), Some(Ok(5)));
    assert_eq!(b.loads, 1);
}

#[test]
fn save_is_snapshot_bound_and_no_saved_on_enqueue() {
    let mut c = Client::new();
    let mut s = Service::new();
    let mut b = fake();
    let mut editor = b"first".to_vec();
    c.load(3, 1);
    assert_eq!(drive(&mut c, &mut s, &mut b).outcome, Outcome::Loaded);
    assert!(c.save(Some(3), [0; 16], &editor, 42));
    editor.copy_from_slice(b"later");
    assert!(c.busy());
    assert!(b.saved.is_empty());
    let e = drive(&mut c, &mut s, &mut b);
    assert_eq!(e.outcome, Outcome::Saved);
    assert_eq!(e.revision, 42);
    assert_eq!(b.saved, vec![b"first".to_vec()]);
    assert_eq!(editor, b"later");
}
#[test]
fn save_as_adopts_new_identity_only_after_complete_commit() {
    let mut c = Client::new();
    let mut b = fake();
    assert!(c.save(None, *b"second\0\0\0\0\0\0\0\0\0\0", b"different", 8));
    let e = drive(&mut c, &mut Service::new(), &mut b);
    assert_eq!(e.outcome, Outcome::Saved);
    assert_eq!(e.object, 5);
    assert_eq!(b.saved, vec![b"different".to_vec()]);
}
#[test]
fn cancelled_upload_has_no_save_effect_and_late_receipt_cannot_complete() {
    let mut c = Client::new();
    let mut s = Service::new();
    let mut b = fake();
    c.save(Some(3), [0; 16], b"new", 4);
    let mut r = None;
    c.step(|x| {
        r = Some(x);
        Ok(())
    });
    let old = rpc(&mut s, &mut b, 9, r.unwrap());
    c.cancel();
    assert!(c.receive(11, old).is_none());
    assert_eq!(drive(&mut c, &mut s, &mut b).outcome, Outcome::Cancelled);
    assert!(b.saved.is_empty());
}

#[test]
fn recycled_client_selection_cannot_be_rebound_by_fresh_info() {
    let mut c = Client::new();
    let mut s = Service::new();
    let mut b = fake();
    c.load(3, 1);
    assert_eq!(drive(&mut c, &mut s, &mut b).outcome, Outcome::Loaded);
    b.generation += 1;
    c.save(Some(3), [0; 16], b"do not overwrite", 2);
    let e = drive(&mut c, &mut s, &mut b);
    assert_eq!(e.outcome, Outcome::Uncertain);
    assert_eq!(e.error, w::STALE);
    assert!(b.saved.is_empty());
}

#[test]
fn deferred_commit_yields_to_abort_and_never_replays_effects() {
    struct Slow {
        turns: usize,
        cancels: usize,
        effects: usize,
    }
    impl Backend for Slow {
        fn identity(&mut self, _: u8) -> Result<u32, u32> {
            Ok(7)
        }
        fn name(&mut self, _: u8) -> Result<[u8; 16], u32> {
            Ok([0; 16])
        }
        fn load(&mut self, _: u8, _: &mut [u8; w::MAX]) -> Result<usize, u32> {
            unreachable!()
        }
        fn save_step(&mut self, _: u32, _: u8, bytes: &[u8]) -> Result<bool, u32> {
            assert_eq!(bytes, b"hi");
            self.turns += 1;
            if self.turns == 4 {
                self.effects += 1;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        fn cancel_save(&mut self, _: u32) {
            self.cancels += 1;
        }
    }
    let mut b = Slow {
        turns: 0,
        cancels: 0,
        effects: 0,
    };
    let mut s = Service::new();
    let req = |seq, kind, a, b| w::Request {
        seq,
        kind,
        object: 3,
        offset: 0,
        a,
        b,
    };
    s.handle(9, true, req(1, w::BEGIN_SAVE, 7, 2), &mut b);
    s.poll(|_, _| 0);
    s.handle(
        9,
        true,
        req(2, w::WRITE, u16::from_le_bytes(*b"hi") as u64, 0),
        &mut b,
    );
    s.poll(|_, _| 0);
    let commit = req(3, w::COMMIT, w::hash(b"hi"), 0);
    s.handle(9, true, commit, &mut b);
    s.poll(|_, _| panic!("admission is not completion"));
    s.progress(&mut b);
    s.handle(9, true, commit, &mut b);
    s.poll(|_, _| panic!("duplicate must not replay or report Saved"));
    assert_eq!(b.turns, 1);
    let abort = w::Request {
        seq: 4,
        kind: w::ABORT,
        object: 0,
        offset: 0,
        a: 0,
        b: 0,
    };
    s.handle(9, true, abort, &mut b);
    s.poll(|_, v| {
        assert_eq!(w::decode_reply(v, 4), Some(Ok(0)));
        0
    });
    s.progress(&mut b);
    assert_eq!((b.turns, b.effects, b.cancels), (1, 0, 1));
    s.retire(9, &mut b);
    s.progress(&mut b);
    assert_eq!(b.turns, 1);
}

#[test]
fn completed_deferred_commit_retains_full_receipt_and_retires_without_replay() {
    let mut s = Service::new();
    let mut b = fake();
    let req = |seq, kind, a, b| w::Request {
        seq,
        kind,
        object: 3,
        offset: 0,
        a,
        b,
    };
    rpc(&mut s, &mut b, 9, req(1, w::BEGIN_SAVE, 7, 2));
    rpc(
        &mut s,
        &mut b,
        9,
        req(2, w::WRITE, u16::from_le_bytes(*b"hi") as u64, 0),
    );
    let r = req(3, w::COMMIT, w::hash(b"hi"), 0);
    s.handle(9, true, r, &mut b);
    assert!(b.saved.is_empty());
    s.progress(&mut b);
    assert_eq!(b.saved, vec![b"hi".to_vec()]);
    for _ in 0..3 {
        s.poll(|pd, v| {
            assert_eq!(pd, 9);
            assert_eq!(w::decode_reply(v, 3), Some(Ok(2)));
            1
        });
        s.progress(&mut b);
    }
    assert_eq!(b.saved.len(), 1);
    s.poll(|_, _| 0);
    assert_eq!(w::decode_reply(rpc(&mut s, &mut b, 9, r), 3), Some(Ok(2)));
    assert_eq!(b.saved.len(), 1);
    s.retire(9, &mut b);
    s.progress(&mut b);
    s.poll(|_, _| panic!("retired receipt"));
}

#[test]
fn third_session_commit_and_retained_reply_use_exact_new_caller() {
    struct Third{steps:usize,cancels:Vec<u32>}
    impl Backend for Third {
        fn identity(&mut self,_:u8)->Result<u32,u32>{Ok(7)}
        fn load(&mut self,_:u8,_:&mut[u8;w::MAX])->Result<usize,u32>{Ok(0)}
        fn name(&mut self,_:u8)->Result<[u8;16],u32>{Ok([0;16])}
        fn save_step(&mut self,pd:u32,_:u8,bytes:&[u8])->Result<bool,u32>{assert_eq!(pd,18);assert_eq!(bytes,b"hi");self.steps+=1;Ok(self.steps==2)}
        fn cancel_save(&mut self,pd:u32){self.cancels.push(pd);}
    }
    let mut s=Service::new();let mut b=Third{steps:0,cancels:vec![]};
    let r=|seq,kind,a,b|w::Request{seq,kind,object:3,offset:0,a,b};
    for req in [r(1,w::BEGIN_SAVE,7,2),r(2,w::WRITE,u16::from_le_bytes(*b"hi")as u64,0)] {
        s.handle(18,true,req,&mut b);s.poll(|pd,_|{assert_eq!(pd,18);0});
    }
    s.handle(18,true,r(3,w::COMMIT,w::hash(b"hi"),0),&mut b);
    s.progress(&mut b);s.poll(|_,_|panic!("early completion"));s.progress(&mut b);
    for _ in 0..4{s.poll(|pd,v|{assert_eq!(pd,18);assert_eq!(w::decode_reply(v,3),Some(Ok(2)));1});}
    assert_eq!(b.steps,2);assert_eq!(b.cancels,vec![18]);
    s.retire(18,&mut b);s.poll(|_,_|panic!("retired receipt"));s.progress(&mut b);
    assert_eq!(b.steps,2);assert_eq!(w::decode_reply(s.handle(18,true,r(4,w::INFO,0,0),&mut b).unwrap(),4),Some(Err(w::DENIED)));
    s.handle(16,true,r(1,w::INFO,0,0),&mut b);s.poll(|pd,_|{assert_eq!(pd,16);0});
}

// Production receipt queue under the actual legacy reply Full value (1).
struct WindowBackend {
    data: Vec<u8>,
    creates: usize,
    saves: Vec<(u32, Vec<u8>)>,
    cancelled: Vec<u32>,
}
impl WindowBackend {
    fn new() -> Self {
        Self { data: (0..97).collect(), creates: 0, saves: vec![], cancelled: vec![] }
    }
}
impl Backend for WindowBackend {
    fn identity(&mut self, _: u8) -> Result<u32,u32> { Ok(7) }
    fn load(&mut self, _: u8, out: &mut [u8;w::MAX]) -> Result<usize,u32> {
        out[..self.data.len()].copy_from_slice(&self.data); Ok(self.data.len())
    }
    fn name(&mut self, _:u8)->Result<[u8;16],u32>{Ok([0;16])}
    fn create(&mut self,_:[u8;16])->Result<u8,u32>{self.creates+=1;Ok(5)}
    fn save(&mut self,pd:u32,_:u8,bytes:&[u8])->Result<(),u32>{self.saves.push((pd,bytes.to_vec()));Ok(())}
    fn cancel_save(&mut self,pd:u32){self.cancelled.push(pd);}
}
fn wr(seq:u32,kind:u8,offset:u32,a:u64,b:u64)->w::Request {
    w::Request { seq,kind,object:if kind==w::ABORT{0}else{3},offset,a,b }
}
fn window_rpc(s:&mut Service,b:&mut WindowBackend,pd:u32,r:w::Request)->u64 {
    if let Some(v)=s.handle(pd,true,r,b){return v;}
    s.progress(b);let mut found=None;s.poll(|p,v|{if p==pd{assert!(found.is_none());found=Some(v);0}else{1}});found.unwrap()
}
#[test]
fn four_owned_receipts_survive_every_partial_drain_and_duplicate() {
    for delivered in 0..=4 {
        let mut s=Service::new();let mut b=WindowBackend::new();
        assert_eq!(w::decode_reply(window_rpc(&mut s,&mut b,9,wr(1,w::LOAD,0,7,0)),1),Some(Ok(97)));
        let requests:Vec<_>=(0..4).map(|i|wr(2+i,w::READ,i*4,4,0)).collect();
        for &r in &requests {assert!(s.handle(9,true,r,&mut b).is_none());}
        for &r in requests.iter().rev(){assert!(s.handle(9,true,r,&mut b).is_none());}
        for _ in 0..3 {let mut calls=0;s.poll(|pd,v|{calls+=1;assert_eq!(pd,9);assert_eq!(w::decode_reply(v,2),Some(Ok(0x03020100)));1});assert_eq!(calls,1);}
        let mut sent=vec![];s.poll(|_,v|{if sent.len()==delivered{return 1;}sent.push(v);0});
        let mut rest=vec![];s.poll(|_,v|{rest.push(v);0});sent.extend(rest);
        assert_eq!(sent.len(),4);assert!(!s.pending());
        for (i,v) in sent.into_iter().enumerate(){let base=i as u8*4;assert_eq!(w::decode_reply(v,2+i as u32),Some(Ok(u32::from_le_bytes([base,base+1,base+2,base+3]))));}
    }
}
#[test]
fn saturated_receipts_reject_fifth_effect_and_commit_before_execution() {
    let mut s=Service::new();let mut b=WindowBackend::new();
    window_rpc(&mut s,&mut b,9,wr(1,w::BEGIN_SAVE,0,7,80));
    for i in 0..4 {s.handle(9,true,wr(2+i,w::WRITE,16*i,0,0),&mut b);}
    let fifth=wr(6,w::WRITE,64,0,0);
    assert_eq!(w::decode_reply(s.handle(9,true,fifth,&mut b).unwrap(),6),Some(Err(w::BUSY)));
    let commit=wr(7,w::COMMIT,0,w::hash(&[0;80]),0);
    assert_eq!(w::decode_reply(s.handle(9,true,commit,&mut b).unwrap(),7),Some(Err(w::BUSY)));
    s.progress(&mut b);assert!(b.saves.is_empty());
    s.poll(|_,_|0);
    // A rejected fifth WRITE did not advance the strict staging offset.
    assert_eq!(w::decode_reply(window_rpc(&mut s,&mut b,9,fifth),6),Some(Ok(16)));
    assert_eq!(w::decode_reply(window_rpc(&mut s,&mut b,9,commit),7),Some(Ok(80)));
    assert_eq!(b.saves,vec![(9,vec![0;80])]);
}
#[test]
fn queued_create_duplicates_never_repeat_backend_effect() {
    let mut s=Service::new();let mut b=WindowBackend::new();
    let req=|seq|w::Request{seq,kind:w::CREATE,object:0,offset:0,a:u64::from_le_bytes(*b"note\0\0\0\0"),b:0};
    for i in 1..=4 {s.handle(9,true,req(i),&mut b);}
    for i in 1..=4 {s.handle(9,true,req(i),&mut b);}
    assert_eq!(b.creates,4);
    assert_eq!(w::decode_reply(s.handle(9,true,req(5),&mut b).unwrap(),5),Some(Err(w::BUSY)));
    assert_eq!(b.creates,4);s.poll(|_,_|0);
    assert_eq!(w::decode_reply(window_rpc(&mut s,&mut b,9,req(1)),1),Some(Err(w::STALE)));
    assert_eq!(b.creates,4);
}
#[test]
fn full_client_does_not_block_other_exact_incarnations() {
    let mut s=Service::new();let mut b=WindowBackend::new();
    for pd in [9,16,18] {for seq in 1..=4 {s.handle(pd,true,wr(seq,w::INFO,0,0,0),&mut b);}}
    let mut counts=[0;3];s.poll(|pd,v|{let i=[9,16,18].iter().position(|p|*p==pd).unwrap();counts[i]+=1;assert_eq!(w::decode_reply(v,counts[i]),Some(Ok(7)));if pd==9{1}else{0}});
    assert_eq!(counts,[1,4,4]);assert!(s.pending());
    let mut left=0;s.poll(|pd,_|{assert_eq!(pd,9);left+=1;0});assert_eq!(left,4);assert!(!s.pending());
}
#[test]
fn fresh_exact_abort_supersedes_four_receipts_and_scrubs_staging() {
    let mut s=Service::new();let mut b=WindowBackend::new();
    window_rpc(&mut s,&mut b,18,wr(1,w::BEGIN_SAVE,0,7,64));
    for i in 0..4{s.handle(18,true,wr(2+i,w::WRITE,i*16,u64::MAX,u64::MAX),&mut b);}
    let abort=wr(6,w::ABORT,0,0,0);
    let mut bad=abort;bad.a=1;
    assert_eq!(w::decode_reply(s.handle(18,true,bad,&mut b).unwrap(),6),Some(Err(w::BUSY)));
    assert!(s.handle(18,true,abort,&mut b).is_none());
    for _ in 0..3{s.poll(|pd,v|{assert_eq!(pd,18);assert_eq!(w::decode_reply(v,6),Some(Ok(0)));1});}
    s.poll(|_,_|0);assert_eq!(b.cancelled,vec![18]);s.progress(&mut b);assert!(b.saves.is_empty());
    assert_eq!(w::decode_reply(window_rpc(&mut s,&mut b,18,wr(7,w::COMMIT,0,w::hash(&[255;64]),0)),7),Some(Err(w::STALE)));
}
#[test]
fn retirement_scrubs_every_owned_receipt_without_touching_other_client() {
    let mut s=Service::new();let mut b=WindowBackend::new();
    for pd in [9,16,18]{for seq in 1..=4{s.handle(pd,true,wr(seq,w::INFO,0,0,0),&mut b);}}
    s.retire(16,&mut b);let mut counts=[0;2];s.poll(|pd,_|{counts[if pd==9{0}else{assert_eq!(pd,18);1}]+=1;0});
    assert_eq!(counts,[4,4]);assert!(!s.pending());
    assert_eq!(w::decode_reply(s.handle(16,true,wr(5,w::INFO,0,0,0),&mut b).unwrap(),5),Some(Err(w::DENIED)));
}

fn read_window_ready(c:&mut Client,s:&mut Service,b:&mut WindowBackend)->Vec<w::Request>{
    assert!(c.load(3,99));
    for _ in 0..20 {
        let mut requests=vec![];assert!(c.step(|r|{requests.push(r);Ok(())}).is_none());
        if requests.first().is_some_and(|r|r.kind==w::READ){assert_eq!(requests.len(),4);return requests;}
        assert_eq!(requests.len(),1);let r=requests[0];let v=window_rpc(s,b,9,r);assert!(c.receive(11,v).is_none());
    }
    panic!("READ window never entered")
}
#[test]
fn read_window_reorders_receipts_places_exact_offsets_and_waits_for_end(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    assert!(c.load(3,42));let mut reads=vec![];let mut max_batch=0;
    for _ in 0..100 {
        let mut requests=vec![];assert!(c.step(|r|{requests.push(r);Ok(())}).is_none());max_batch=max_batch.max(requests.len());
        for r in requests {
            if r.kind==w::READ{reads.push(r);}
            if r.kind==w::END{
                // Every data ACK has arrived, but no Loaded before the barrier.
                assert_eq!(&c.bytes[..c.len],&b.data);assert!(c.busy());
                let v=window_rpc(&mut s,&mut b,9,r);let e=c.receive(11,v).unwrap();
                assert_eq!(e.outcome,Outcome::Loaded);assert_eq!(e.revision,42);
                assert_eq!(reads.len(),25);assert_eq!(max_batch,4);return;
            }
            assert!(s.handle(9,true,r,&mut b).is_none());
        }
        for _ in 0..3{s.poll(|_,_|1);}
        let mut replies=vec![];s.poll(|pd,v|{assert_eq!(pd,9);replies.push(v);0});
        for v in replies.into_iter().rev(){assert!(c.receive(11,v).is_none());assert!(c.receive(11,v).is_none());}
    }
    panic!("load did not reach END")
}
#[test]
fn wrong_or_duplicate_read_receipt_never_frees_window_credit(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=read_window_ready(&mut c,&mut s,&mut b);
    for &r in &requests{assert!(s.handle(9,true,r,&mut b).is_none());}
    let mut replies=vec![];s.poll(|_,v|{replies.push(v);0});
    c.receive(16,replies[0]);c.receive(11,w::reply(requests[3].seq+1,Ok(0)));c.receive(11,0);
    c.step(|_|panic!("wrong reply freed credit"));
    c.receive(11,replies[0]);let mut refill=vec![];c.step(|r|{refill.push(r);Ok(())});
    assert_eq!(refill.len(),1);assert_eq!(refill[0].offset,16);
    c.receive(11,replies[0]);c.step(|_|panic!("duplicate freed credit"));
}
#[test]
fn read_retry_only_resends_definitely_not_enqueued_descriptor(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let first=read_window_ready(&mut c,&mut s,&mut b);
    for r in first{let v=window_rpc(&mut s,&mut b,9,r);c.receive(11,v);}
    let mut attempted=vec![];c.step(|r|{attempted.push(r);if attempted.len()==2{Err(ERR_SERVICE_NOT_READY)}else{Ok(())}});
    assert_eq!(attempted.len(),2);let accepted=attempted[0];let retry=attempted[1];
    let mut next=vec![];c.step(|r|{next.push(r);Ok(())});assert_eq!(next[0],retry);assert_eq!(next.len(),3);assert!(!next.contains(&accepted));
    c.step(|_|panic!("accepted read was retried"));
}
#[test]
fn cancel_fences_all_read_descriptors_and_outranks_saturated_receipts(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=read_window_ready(&mut c,&mut s,&mut b);
    for &r in &requests{s.handle(9,true,r,&mut b);}
    assert!(c.cancel());for r in &requests{assert!(c.receive(11,w::reply(r.seq,Ok(0))).is_none());}
    let mut abort=None;c.step(|r|{abort=Some(r);Ok(())});let abort=abort.unwrap();
    assert_eq!(abort.kind,w::ABORT);assert!(abort.seq>requests.last().unwrap().seq);
    assert!(s.handle(9,true,abort,&mut b).is_none());let mut replies=vec![];s.poll(|_,v|{replies.push(v);0});assert_eq!(replies.len(),1);
    assert_eq!(c.receive(11,replies[0]).unwrap().outcome,Outcome::Cancelled);
    assert!(c.bytes.iter().all(|&v|v==0));assert!(c.load(3,100));
}
#[test]
fn missing_read_receipt_expires_despite_forged_progress(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=read_window_ready(&mut c,&mut s,&mut b);
    let mut event=None;
    for _ in 0..1_048_576{
        c.receive(9,w::reply(requests[0].seq,Ok(0)));c.receive(11,w::reply(requests[3].seq+1,Ok(0)));
        event=c.step(|_|panic!("missing accepted request was retried"));if event.is_some(){break;}
    }
    assert_eq!(event.unwrap().outcome,Outcome::Uncertain);assert_eq!(c.len,0);assert!(!c.load(3,100));
}
#[test]
fn nonzero_read_tail_fails_without_publishing_partial_data(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    assert!(c.load(3,42));
    for _ in 0..100 {
        let mut requests=vec![];c.step(|r|{requests.push(r);Ok(())});
        for r in requests {
            if r.kind==w::READ&&r.a<4{
                let e=c.receive(11,w::reply(r.seq,Ok(0x100))).unwrap();assert_eq!(e.outcome,Outcome::Uncertain);assert_eq!(e.error,w::INVALID);assert_eq!(c.len,0);assert!(c.bytes.iter().all(|&v|v==0));return;
            }
            let v=window_rpc(&mut s,&mut b,9,r);c.receive(11,v);
        }
    }
    panic!("tail not reached")
}
#[test]
fn close_scrubs_four_reads_and_late_replies_cannot_revive_client(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=read_window_ready(&mut c,&mut s,&mut b);c.close();
    for r in requests{assert!(c.receive(11,w::reply(r.seq,Ok(0xffffffff))).is_none());}
    assert!(c.bytes.iter().all(|&v|v==0));assert_eq!(c.len,0);
    c.step(|_|panic!("closed client issued work"));assert!(!c.load(3,100));assert!(!c.cancel());
}
#[path="../kernel/src/ipc_ring.rs"]mod production_ring;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../servers/sexfiles/src/defer_queue.rs"]mod production_deferred;
fn queued_document_drive(c:&mut Client,s:&mut Service,b:&mut WindowBackend)->(client::Event,Vec<w::Request>){
    let ring=production_ring::RingBuffer::<production_deferred::Message,4>::new();
    let replies=production_ring::RingBuffer::<u64,2>::new();
    let mut deferred=production_deferred::Deferred::new();let mut seen=vec![];
    for i in 0..8{assert!(deferred.push((0x99,12,i,0,0)));}
    for _ in 0..10000{
        if let Some(e)=c.step(|r|{
            ring.enqueue((w::OP,9,r.header(),r.a,r.b)).map_err(|_|ERR_SERVICE_NOT_READY)?;
            assert!(!seen.iter().any(|old:&w::Request|old.seq==r.seq),"accepted mutation retried");seen.push(r);Ok(())
        }){return(e,seen);}
        while let Some(m)=ring.dequeue(){assert!(deferred.push(m));}
        while let Some(m)=deferred.pop(){if m.0==w::OP{let r=w::Request::decode(m.2,m.3,m.4).unwrap();assert!(s.handle(m.1,true,r,b).is_none());}}
        s.progress(b);
        s.poll(|pd,v|{assert_eq!(pd,9);if replies.enqueue(v).is_ok(){0}else{1}});
        let mut batch=vec![];while let Some(v)=replies.dequeue(){batch.push(v);}
        // Kernel request order stays FIFO; client receipt correlation also
        // remains correct when the host delivery model reverses ACK batches.
        for v in batch.into_iter().rev(){if let Some(e)=c.receive(11,v){return(e,seen);}}
    }
    panic!("queued document exchange did not terminate")
}
#[test]
fn actual_ring_and_deferred_fifo_round_trip_all_tails_and_maximum(){
    for len in [0,1,3,4,5,15,16,17,63,64,65,w::MAX]{
        let mut b=WindowBackend::new();b.data=(0..len).map(|i|(i*31+7)as u8).collect();
        let mut s=Service::new();let mut c=Client::new();assert!(c.load(3,12));
        let (e,reads)=queued_document_drive(&mut c,&mut s,&mut b);assert_eq!(e.outcome,Outcome::Loaded);assert_eq!(&c.bytes[..c.len],&b.data);
        let reads:Vec<_>=reads.into_iter().filter(|r|r.kind==w::READ).collect();assert_eq!(reads.len(),len.div_ceil(4));
        for (i,r)in reads.iter().enumerate(){assert_eq!(r.offset as usize,i*4);assert_eq!(r.a as usize,(len-i*4).min(4));}
        let frozen:Vec<_>=(0..len).map(|i|(i*17+9)as u8).collect();assert!(c.save(Some(3),[0;16],&frozen,13));
        let (e,writes)=queued_document_drive(&mut c,&mut s,&mut b);assert_eq!(e.outcome,Outcome::Saved);assert_eq!(e.revision,13);assert_eq!(b.saves,vec![(9,frozen)]);
        let writes:Vec<_>=writes.into_iter().filter(|r|r.kind==w::WRITE).collect();assert_eq!(writes.len(),len.div_ceil(16));
        for (i,r)in writes.iter().enumerate(){assert_eq!(r.offset as usize,i*16);}
        assert!(!s.pending());
    }
}
fn write_window_ready(c:&mut Client,s:&mut Service,b:&mut WindowBackend)->Vec<w::Request>{
    assert!(c.save(Some(2),[0;16],&(0..97).collect::<Vec<u8>>(),55));
    for _ in 0..10{
        let mut requests=vec![];c.step(|r|{requests.push(r);Ok(())});
        if requests[0].kind==w::WRITE{assert_eq!(requests.len(),4);return requests;}
        assert_eq!(requests.len(),1);let v=window_rpc(s,b,9,requests[0]);c.receive(11,v);
    }
    panic!("WRITE not reached")
}
#[test]
fn write_window_holds_commit_until_every_exact_length_ack(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let mut requests=write_window_ready(&mut c,&mut s,&mut b);let mut held=None;
    loop {
        for r in requests.drain(..){assert_eq!(r.kind,w::WRITE);s.handle(9,true,r,&mut b);}
        let mut receipts=vec![];s.poll(|_,v|{receipts.push(v);0});
        for v in receipts{if held.is_none(){held=Some(v);}else{assert!(c.receive(11,v).is_none());}}
        c.step(|r|{requests.push(r);Ok(())});if requests.is_empty(){break;}
    }
    assert!(b.saves.is_empty());c.step(|_|panic!("COMMIT before all WRITE ACKs"));
    c.receive(16,held.unwrap());c.step(|_|panic!("wrong sender freed COMMIT"));
    c.receive(11,held.unwrap());let mut commit=None;c.step(|r|{commit=Some(r);Ok(())});let commit=commit.unwrap();assert_eq!(commit.kind,w::COMMIT);
    let v=window_rpc(&mut s,&mut b,9,commit);assert!(c.receive(11,v).is_none());assert!(c.busy());
    let mut end=None;c.step(|r|{end=Some(r);Ok(())});let end=end.unwrap();assert_eq!(end.kind,w::END);
    assert_eq!(c.receive(11,window_rpc(&mut s,&mut b,9,end)).unwrap().outcome,Outcome::Saved);
    assert_eq!(b.saves,vec![(9,(0..97).collect())]);
}
#[test]
fn partial_write_enqueue_full_retries_only_unsent_request_in_order(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=write_window_ready(&mut c,&mut s,&mut b);for r in requests{c.receive(11,window_rpc(&mut s,&mut b,9,r));}
    let mut tried=vec![];c.step(|r|{tried.push(r);if tried.len()==2{Err(ERR_SERVICE_NOT_READY)}else{Ok(())}});assert_eq!(tried.len(),2);
    let accepted=tried[0];assert_eq!(accepted.offset,64);let retry=tried[1];assert_eq!(retry.offset,80);
    let mut next=vec![];c.step(|r|{next.push(r);Ok(())});assert_eq!(next[0],retry);assert_eq!(next.len(),2);assert_eq!(next[1].offset,96);
    c.step(|_|panic!("accepted WRITE retried"));assert!(b.saves.is_empty());
}
#[test]
fn write_cancel_is_idempotent_and_clears_staged_uncommitted_window(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=write_window_ready(&mut c,&mut s,&mut b);for &r in &requests{s.handle(9,true,r,&mut b);}
    assert!(c.cancel());let mut abort=None;c.step(|r|{abort=Some(r);Ok(())});let abort=abort.unwrap();assert_eq!(abort.kind,w::ABORT);
    for _ in 0..100{assert!(c.cancel());c.step(|_|panic!("repeated cancel allocated another ABORT"));}
    s.handle(9,true,abort,&mut b);let mut ack=None;s.poll(|_,v|{assert!(ack.is_none());ack=Some(v);0});
    for r in requests{assert!(c.receive(11,w::reply(r.seq,Ok(16))).is_none());}
    assert_eq!(c.receive(11,ack.unwrap()).unwrap().outcome,Outcome::Cancelled);assert!(b.saves.is_empty());
}
#[test]
fn malformed_write_ack_fences_without_automatic_commit_or_retry(){
    let mut c=Client::new();let mut s=Service::new();let mut b=WindowBackend::new();
    let requests=write_window_ready(&mut c,&mut s,&mut b);let e=c.receive(11,w::reply(requests[0].seq,Ok(15))).unwrap();
    assert_eq!(e.outcome,Outcome::Uncertain);assert_eq!(e.error,w::INVALID);c.step(|_|panic!("uncertain mutation replayed"));assert!(b.saves.is_empty());
}
