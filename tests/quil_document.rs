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
        let mut r = None;
        if let Some(e) = c.step(|x| {
            r = Some(x);
            Ok(())
        }) {
            return e;
        }
        if let Some(r) = r {
            let v = rpc(s, b, 9, r);
            if let Some(e) = c.receive(11, v) {
                return e;
            }
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
