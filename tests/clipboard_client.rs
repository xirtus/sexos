//! Actual client/ABI, a finite malformed peer, and actual broker integration.
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/text_encoding.rs"] pub mod text_encoding;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
mod app_lifecycle;
#[path = "../servers/silk-shell/src/clipboard.rs"]
mod broker;
#[path = "../crates/sex-pdx/src/clipboard.rs"]
mod clipboard;
#[path = "../crates/sex-pdx/src/clipboard_client.rs"]
mod clipboard_client;
#[path = "../crates/sex-pdx/src/text_selection.rs"]
mod text_selection;
use clipboard::{
    Action, Packet, Participant, Phase, Request, Response, Status, Subject, MAX_BYTES, SHELL,
};
use clipboard_client::{Admission, Client, Event, View};
use text_selection::Source;
const SOURCE: Source = Source {
    generation: 17,
    revision: 0,
};
const PARTICIPANT: Participant = Participant {
    subject: Subject { pd: 9, task: 701 },
    surface: 156,
    route: 41,
    can_read: true,
};
fn client() -> Client {
    let mut c = Client::new();
    assert!(c.bind(PARTICIPANT));
    c
}
fn deliver(c: &mut Client, r: Response, now: u64) -> bool {
    let p = r.wire();
    c.receive(SHELL, p.opcode, p.args, now)
}
fn intent(c: &mut Client, ticket: u32, action: Action) {
    assert!(deliver(c, Response::Intent { ticket, action }, 1));
}
fn meta(ticket: u32, phase: Phase, len: usize, generation: u32, hash: u64) -> Response {
    Response::Meta {
        ticket,
        phase,
        status: Status::Ok,
        len,
        generation,
        hash,
    }
}
fn send(c: &mut Client, now: u64) -> Vec<Request> {
    let mut requests = vec![];
    c.poll(now, |p| {
        requests.push(Request::decode(p.opcode, p.args).unwrap());
        Admission::Accepted
    });
    requests
}
struct Peer {
    body: Vec<u8>,
    staging: Vec<u8>,
    hash: u64,
    generation: u32,
    log: Vec<Request>,
}
impl Peer {
    fn new(body: &[u8]) -> Self {
        Self {
            body: body.to_vec(),
            staging: vec![],
            hash: clipboard::hash(body),
            generation: 55,
            log: vec![],
        }
    }
    fn request(&mut self, r: Request) -> Option<Response> {
        self.log.push(r);
        Some(match r {
            Request::Begin { ticket, len, .. } => {
                self.staging.clear();
                meta(ticket, Phase::Begin, len, 0, 0)
            }
            Request::Write {
                offset,
                count,
                bytes,
                ..
            } => {
                assert_eq!(offset, self.staging.len());
                self.staging.extend_from_slice(&bytes[..count]);
                return None;
            }
            Request::Publish { ticket, len, hash } => {
                assert_eq!(len, self.staging.len());
                assert_eq!(hash, clipboard::hash(&self.staging));
                self.body = self.staging.clone();
                self.hash = hash;
                meta(ticket, Phase::Publish, len, self.generation, hash)
            }
            Request::Read { ticket } => meta(
                ticket,
                Phase::Read,
                self.body.len(),
                self.generation,
                self.hash,
            ),
            Request::Get { ticket, offset } => {
                let count = (self.body.len() - offset).min(16);
                let mut bytes = [0; 16];
                bytes[..count].copy_from_slice(&self.body[offset..offset + count]);
                Response::Data {
                    ticket,
                    offset,
                    count,
                    bytes,
                }
            }
            Request::Finish {
                ticket,
                generation,
                hash,
            } => {
                assert_eq!(generation, self.generation);
                assert_eq!(hash, self.hash);
                meta(
                    ticket,
                    Phase::Finish,
                    self.body.len(),
                    self.generation,
                    self.hash,
                )
            }
            Request::Cancel { ticket } => meta(ticket, Phase::Cancel, 0, 0, 0),
            _ => panic!("unexpected request"),
        })
    }
    fn turn(&mut self, c: &mut Client, now: u64) {
        for r in send(c, now) {
            if let Some(r) = self.request(r) {
                deliver(c, r, now);
            }
        }
    }
}
fn copy(c: &mut Client, peer: &mut Peer, body: &[u8], ticket: u32, cut: bool) -> Event {
    intent(c, ticket, if cut { Action::Cut } else { Action::Copy });
    assert_eq!(c.take_event(), Some(Event::Export { ticket, cut }));
    assert!(c.publish(ticket, SOURCE, body, 1));
    for now in 2..1000 {
        peer.turn(c, now);
        if let Some(event) = c.take_event() {
            return event;
        }
    }
    panic!("copy did not finish")
}
fn paste(c: &mut Client, peer: &mut Peer, ticket: u32) -> Event {
    intent(c, ticket, Action::Paste);
    assert_eq!(c.take_event(), Some(Event::Paste { ticket }));
    assert!(c.read(ticket, 1));
    for now in 2..2000 {
        peer.turn(c, now);
        if let Some(event) = c.take_event() {
            return event;
        }
    }
    panic!("paste did not finish")
}
#[test]
fn complete_12k_copy_cut_and_paste_exact_whole_body() {
    let body: Vec<u8> = (0..MAX_BYTES)
        .map(|i| {
            if i % 61 == 0 {
                b'\n'
            } else {
                b' ' + (i % 95) as u8
            }
        })
        .collect();
    let mut c = client();
    let mut p = Peer::new(&[]);
    let event = copy(&mut c, &mut p, &body, 7, true);
    assert_eq!(
        event,
        Event::Published {
            ticket: 7,
            cut: true,
            len: MAX_BYTES,
            generation: 55,
            hash: clipboard::hash(&body)
        }
    );
    assert_eq!(c.bytes(), None);
    assert!(c.storage_erased());
    assert!(matches!(
        paste(&mut c, &mut p, 8),
        Event::Ready { len: MAX_BYTES, .. }
    ));
    assert_eq!(c.bytes(), Some(body.as_slice()));
    assert!(c.finish_read());
    assert_eq!(c.bytes(), None);
    assert!(c.storage_erased());
    assert_eq!(
        p.log
            .iter()
            .filter(|r| matches!(r, Request::Publish { .. }))
            .count(),
        1
    );
    assert_eq!(
        p.log
            .iter()
            .filter(|r| matches!(r, Request::Finish { .. }))
            .count(),
        1
    );
}
#[test]
fn export_is_immutable_and_begin_and_publish_are_not_replayed() {
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    let mut body = b"original bytes".to_vec();
    assert!(c.publish(1, SOURCE, &body, 1));
    body.fill(b'X');
    let first = send(&mut c, 2);
    assert_eq!(first.len(), 1);
    for n in 3..10 {
        assert!(send(&mut c, n).is_empty())
    }
    deliver(&mut c, meta(1, Phase::Begin, 14, 0, 0), 10);
    let writes = send(&mut c, 11);
    let Request::Write { bytes, count, .. } = writes[0] else {
        panic!()
    };
    assert_eq!(&bytes[..count], b"original bytes");
    assert!(matches!(writes[1], Request::Publish { .. }));
    for n in 12..20 {
        assert!(send(&mut c, n).is_empty())
    }
}
#[test]
fn exact_sender_and_monotonic_intents_bind_once() {
    let mut c = client();
    assert!(!c.bind(Participant {
        subject: Subject { pd: 9, task: 9 },
        ..PARTICIPANT
    }));
    let p = Response::Intent {
        ticket: 4,
        action: Action::Copy,
    }
    .wire();
    for sender in [
        Subject { pd: 3, task: 4 },
        Subject { pd: 4, task: 3 },
        Subject { pd: 0, task: 3 },
    ] {
        assert!(!c.receive(sender, p.opcode, p.args, 1))
    }
    assert_eq!(c.take_event(), None);
    intent(&mut c, 4, Action::Copy);
    c.take_event();
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 5,
            action: Action::Paste
        },
        2
    ));
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 3,
            action: Action::Copy
        },
        2
    ));
    assert!(c.publish(4, SOURCE, b"hello", 2));
    assert!(matches!(
        send(&mut c, 3)[0],
        Request::Begin { ticket: 4, .. }
    ));
}
#[test]
fn read_only_participant_cannot_cut_or_paste() {
    for action in [Action::Cut, Action::Paste] {
        let mut c = Client::new();
        assert!(c.bind(Participant {
            can_read: false,
            ..PARTICIPANT
        }));
        intent(&mut c, 1, action);
        assert_eq!(c.view(), View::Unsupported);
        assert_eq!(send(&mut c, 2), vec![Request::Cancel { ticket: 1 }]);
        assert_eq!(c.take_event(), None)
    }
}
#[test]
fn full_keeps_identical_packet_at_every_phase_and_no_accepted_retry() {
    let mut c = client();
    let mut peer = Peer::new(b"a read body which has two chunks");
    intent(&mut c, 1, Action::Paste);
    c.take_event();
    c.read(1, 1);
    let mut now = 2;
    while c.bytes().is_none() {
        let mut full = None;
        c.poll(now, |p| {
            full = Some(p);
            Admission::Full
        });
        now += 1;
        let mut full2 = None;
        c.poll(now, |p| {
            full2 = Some(p);
            Admission::Full
        });
        assert_eq!(full, full2);
        assert!(full.is_some());
        now += 1;
        let requests = send(&mut c, now);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].wire(), full.unwrap());
        assert!(c.packet_erased());
        now += 1;
        assert!(send(&mut c, now).is_empty());
        let r = peer.request(requests[0]).unwrap();
        deliver(&mut c, r, now);
        now += 1;
    }
    assert_eq!(c.bytes(), Some(peer.body.as_slice()));
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    c.publish(1, SOURCE, b"short text", 1);
    let mut seen = vec![];
    for now in 2..30 {
        let mut packet = None;
        c.poll(now, |p| {
            packet = Some(p);
            Admission::Full
        });
        let Some(p) = packet else { break };
        seen.push(p.opcode);
        let requests = send(&mut c, now + 1);
        assert_eq!(requests.first().unwrap().wire(), p);
        for r in requests {
            if let Some(r) = peer.request(r) {
                deliver(&mut c, r, now + 1);
            }
        }
        if c.view() == View::Copied {
            break;
        }
    }
    assert!(seen.contains(&clipboard::BEGIN));
    assert!(seen.contains(&clipboard::WRITE)); // Publish can share the last accepted Write turn.
}
fn pending_read() -> Client {
    let mut c = client();
    intent(&mut c, 9, Action::Paste);
    c.take_event();
    c.read(9, 1);
    send(&mut c, 2);
    c
}
#[test]
fn read_hidden_until_finish_and_late_retirement_cannot_revoke_handoff() {
    let mut c = pending_read();
    let body = b"paste";
    let hash = clipboard::hash(body);
    deliver(&mut c, meta(9, Phase::Read, 5, 4, hash), 3);
    assert!(c.bytes().is_none());
    send(&mut c, 4);
    let mut bytes = [0; 16];
    bytes[..5].copy_from_slice(body);
    deliver(
        &mut c,
        Response::Data {
            ticket: 9,
            offset: 0,
            count: 5,
            bytes,
        },
        4,
    );
    assert!(c.bytes().is_none());
    send(&mut c, 5);
    assert!(c.bytes().is_none());
    deliver(&mut c, meta(9, Phase::Finish, 5, 4, hash), 6);
    assert_eq!(c.bytes(), Some(&body[..]));
    assert!(!deliver(
        &mut c,
        Response::Meta {
            ticket: 9,
            phase: Phase::Finish,
            status: Status::Invalidated,
            len: 0,
            generation: 0,
            hash: 0
        },
        7
    ));
    assert_eq!(c.bytes(), Some(&body[..]));
}
#[test]
fn invalidated_in_every_read_phase_wipes_and_blocks_application() {
    for phase in 0..4 {
        let mut c = pending_read();
        let body = b"read";
        let h = clipboard::hash(body);
        if phase > 0 {
            deliver(&mut c, meta(9, Phase::Read, 4, 5, h), 3);
        }
        if phase > 1 {
            send(&mut c, 4);
        }
        if phase > 2 {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(body);
            deliver(
                &mut c,
                Response::Data {
                    ticket: 9,
                    offset: 0,
                    count: 4,
                    bytes,
                },
                4,
            );
            send(&mut c, 5);
        }
        deliver(
            &mut c,
            Response::Meta {
                ticket: 9,
                phase: Phase::Finish,
                status: Status::Invalidated,
                len: 0,
                generation: 0,
                hash: 0,
            },
            6,
        );
        assert_eq!(c.view(), View::Uncertain);
        assert!(c.bytes().is_none());
        assert!(c.storage_erased());
        assert!(!deliver(&mut c, meta(9, Phase::Finish, 4, 5, h), 7));
        assert_eq!(
            c.take_event(),
            Some(Event::Stopped {
                ticket: 9,
                view: View::Uncertain
            })
        );
    }
}
#[test]
fn wrong_hash_out_of_order_count_text_and_nonzero_tail_never_ready() {
    for fault in 0..6 {
        let mut c = pending_read();
        let h = clipboard::hash(b"hello");
        deliver(&mut c, meta(9, Phase::Read, 5, 3, h), 3);
        if fault != 5 {
            send(&mut c, 4);
        }
        let mut bytes = [0; 16];
        bytes[..5].copy_from_slice(b"hello");
        if fault == 0 {
            bytes[0] = b'X'
        }
        if fault == 3 {
            bytes[0] = 0
        }
        if fault == 4 {
            bytes[15] = 1
        }
        let p = Response::Data {
            ticket: 9,
            offset: if fault == 1 { 16 } else { 0 },
            count: if fault == 2 { 4 } else { 5 },
            bytes,
        }
        .wire();
        assert!(c.receive(SHELL, p.opcode, p.args, 5));
        assert_eq!(c.view(), View::Uncertain);
        assert!(c.bytes().is_none());
        assert!(c.storage_erased());
        assert!(c.packet_erased());
    }
}
#[test]
fn stale_response_and_foreign_task_dont_advance_or_mutate() {
    let mut c = pending_read();
    for sender in [Subject { pd: 3, task: 99 }, Subject { pd: 99, task: 3 }] {
        let p = meta(9, Phase::Read, 5, 3, 4).wire();
        assert!(!c.receive(sender, p.opcode, p.args, 3));
    }
    assert!(!deliver(&mut c, meta(8, Phase::Read, 5, 3, 4), 3));
    assert!(send(&mut c, 4).is_empty());
    assert!(c.bytes().is_none());
}
#[test]
fn malformed_metadata_alias_and_mismatched_finish_fail_closed() {
    for bad in 0..3 {
        let mut c = pending_read();
        let mut p = meta(9, Phase::Read, 5, 3, 4).wire();
        if bad == 0 {
            p.args[0] |= 1 << 63;
        }
        if bad == 1 {
            p.args[1] |= 1 << 16;
        }
        if bad == 2 {
            p = meta(9, Phase::Read, 0, 0, 0).wire();
        }
        c.receive(SHELL, p.opcode, p.args, 3);
        assert_eq!(c.view(), View::Uncertain);
        assert!(c.storage_erased());
    }
    let mut c = pending_read();
    deliver(&mut c, meta(9, Phase::Publish, 3, 1, 3), 3);
    assert!(send(&mut c, 4).is_empty());
    assert_eq!(c.view(), View::Pending);
}
#[test]
fn cancel_fences_late_finish_and_requires_positive_cancel_receipt() {
    let mut c = pending_read();
    c.cancel(3);
    assert!(c.storage_erased());
    let requests = send(&mut c, 4);
    assert_eq!(requests, vec![Request::Cancel { ticket: 9 }]);
    assert!(!deliver(&mut c, meta(9, Phase::Finish, 5, 3, 4), 5));
    assert_eq!(c.take_event(), None);
    assert!(send(&mut c, 6).is_empty());
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 10,
            action: Action::Copy
        },
        7
    ));
    deliver(&mut c, meta(9, Phase::Cancel, 0, 0, 0), 8);
    assert_eq!(c.view(), View::Cancelled);
    assert_eq!(
        c.take_event(),
        Some(Event::Stopped {
            ticket: 9,
            view: View::Cancelled
        })
    );
    intent(&mut c, 11, Action::Copy);
    assert!(matches!(
        c.take_event(),
        Some(Event::Export { ticket: 11, .. })
    ));
}
#[test]
fn deadline_backwards_frozen_stall_and_total_fail_without_replay() {
    for now in [0, 2002, 15001] {
        let mut c = pending_read();
        assert_eq!(c.poll(now, |_| panic!("replay")), 0);
        assert_eq!(c.view(), View::Uncertain);
        assert!(c.storage_erased());
        assert!(c.packet_erased());
        assert!(!deliver(&mut c, meta(9, Phase::Read, 5, 3, 4), now));
    }
    let mut c = pending_read();
    for _ in 0..clipboard_client::STALL_TURNS {
        c.poll(2, |_| panic!("replay"));
    }
    assert_eq!(c.view(), View::Uncertain);
    // Keep admitting GET/data progress while advancing time slowly: frozen-clock
    // and total-turn bounds still stop an endless sequence of peer activity.
    let mut c = pending_read();
    for n in 0..clipboard_client::STALL_TURNS {
        c.poll(2, |_| Admission::Full);
        if n == 100 {
            deliver(&mut c, meta(9, Phase::Read, MAX_BYTES, 3, 4), 2);
        }
    }
    assert_eq!(c.view(), View::Uncertain);
}
#[test]
fn close_is_permanent_and_erases_pending_packet_and_whole_payload() {
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    c.publish(1, SOURCE, &[b'A'; MAX_BYTES], 1);
    c.poll(2, |_| Admission::Full);
    assert!(!c.packet_erased());
    c.close();
    assert!(c.storage_erased());
    assert!(c.packet_erased());
    assert!(c.bytes().is_none());
    assert!(!c.bind(PARTICIPANT));
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 2,
            action: Action::Paste
        },
        3
    ));
    assert_eq!(c.poll(4, |_| panic!("closed send")), 0);
    assert!(!c.publish(1, SOURCE, b"x", 5));
}
#[test]
fn empty_invalid_and_oversize_exports_are_atomic_and_never_sent() {
    for body in [vec![], vec![0], vec![b'\r'], vec![b'a'; MAX_BYTES + 1]] {
        let mut c = client();
        intent(&mut c, 1, Action::Copy);
        c.take_event();
        assert!(!c.publish(1, SOURCE, &body, 1));
        assert!(c.storage_erased());
        assert_eq!(send(&mut c, 2), vec![Request::Cancel { ticket: 1 }]);
        deliver(&mut c, meta(1, Phase::Cancel, 0, 0, 0), 3);
        assert!(matches!(c.view(), View::Empty | View::Unsupported));
    }
}
#[test]
fn total_deadline_bounds_sustained_forward_progress_without_stall() {
    let mut c = pending_read();
    let body = vec![b'a'; MAX_BYTES];
    let h = clipboard::hash(&body);
    deliver(&mut c, meta(9, Phase::Read, MAX_BYTES, 3, h), 2);
    for turn in 0..70000u64 {
        let now = 3 + turn / 128;
        if turn % 100 == 0 {
            for r in send(&mut c, now) {
                if let Request::Get { ticket, offset } = r {
                    let mut bytes = [0; 16];
                    bytes.copy_from_slice(&body[offset..offset + 16]);
                    deliver(
                        &mut c,
                        Response::Data {
                            ticket,
                            offset,
                            count: 16,
                            bytes,
                        },
                        now,
                    );
                } else {
                    panic!("total turn bound missed")
                }
            }
        } else {
            c.poll(now, |_| Admission::Full);
        }
        if c.view() == View::Uncertain {
            assert!(turn >= 65000);
            assert!(turn < 66000);
            assert!(c.storage_erased());
            return;
        }
    }
    panic!("total deadline failed")
}
#[test]
fn total_tick_deadline_bounds_progress_that_never_stalls() {
    let mut c = pending_read();
    let body = vec![b'a'; MAX_BYTES];
    let h = clipboard::hash(&body);
    deliver(&mut c, meta(9, Phase::Read, MAX_BYTES, 3, h), 2);
    for turn in 1..20u64 {
        let now = 2 + turn * 1000;
        for r in send(&mut c, now) {
            let Request::Get { ticket, offset } = r else {
                panic!()
            };
            deliver(
                &mut c,
                Response::Data {
                    ticket,
                    offset,
                    count: 16,
                    bytes: [b'a'; 16],
                },
                now,
            );
        }
        if c.view() == View::Uncertain {
            assert_eq!(turn, 15);
            assert!(c.storage_erased());
            return;
        }
    }
    panic!("total tick deadline failed")
}
#[test]
fn publish_and_cancel_full_packets_are_retained_then_admitted_once() {
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    c.publish(1, SOURCE, b"x", 1);
    send(&mut c, 2);
    deliver(&mut c, meta(1, Phase::Begin, 1, 0, 0), 3);
    let mut write = None;
    let mut publish = None;
    let mut index = 0;
    c.poll(4, |p| {
        index += 1;
        if index == 1 {
            write = Some(p);
            Admission::Accepted
        } else {
            publish = Some(p);
            Admission::Full
        }
    });
    assert_eq!(write.unwrap().opcode, clipboard::WRITE);
    assert_eq!(publish.unwrap().opcode, clipboard::PUBLISH);
    for now in 5..9 {
        c.poll(now, |p| {
            assert_eq!(Some(p), publish);
            Admission::Full
        });
    }
    assert_eq!(send(&mut c, 9)[0].wire(), publish.unwrap());
    assert!(c.packet_erased());
    assert!(send(&mut c, 10).is_empty());
    c.cancel(11);
    let mut cancel = None;
    c.poll(12, |p| {
        cancel = Some(p);
        Admission::Full
    });
    c.poll(13, |p| {
        assert_eq!(Some(p), cancel);
        Admission::Full
    });
    assert_eq!(send(&mut c, 14)[0].wire(), cancel.unwrap());
    assert!(send(&mut c, 15).is_empty());
    assert!(c.packet_erased());
}
#[test]
fn positive_finish_requires_identical_length_generation_and_hash() {
    for fault in 0..3 {
        let mut c = pending_read();
        let h = clipboard::hash(b"x");
        deliver(&mut c, meta(9, Phase::Read, 1, 7, h), 3);
        send(&mut c, 4);
        let mut bytes = [0; 16];
        bytes[0] = b'x';
        deliver(
            &mut c,
            Response::Data {
                ticket: 9,
                offset: 0,
                count: 1,
                bytes,
            },
            4,
        );
        send(&mut c, 5);
        deliver(
            &mut c,
            meta(
                9,
                Phase::Finish,
                if fault == 0 { 2 } else { 1 },
                if fault == 1 { 8 } else { 7 },
                if fault == 2 { h ^ 1 } else { h },
            ),
            6,
        );
        assert_eq!(c.view(), View::Uncertain);
        assert!(c.storage_erased());
        assert!(c.bytes().is_none());
    }
}
#[test]
fn terminal_event_is_retained_until_consumed_and_decline_quiesces_intent() {
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    assert!(c.decline(1, View::Cancelled, 2));
    assert_eq!(send(&mut c, 3), vec![Request::Cancel { ticket: 1 }]);
    deliver(&mut c, meta(1, Phase::Cancel, 0, 0, 0), 4);
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 2,
            action: Action::Copy
        },
        5
    ));
    assert_eq!(
        c.take_event(),
        Some(Event::Stopped {
            ticket: 1,
            view: View::Cancelled
        })
    );
    intent(&mut c, 3, Action::Copy);
    c.take_event();
    assert!(!c.publish(
        3,
        Source {
            generation: 0,
            revision: 0
        },
        b"valid bytes",
        1
    ));
    assert_eq!(send(&mut c, 2), vec![Request::Cancel { ticket: 3 }]);
    assert!(c.storage_erased());
}
#[test]
fn admission_rejection_cannot_replay_accepted_mutation() {
    let mut c = pending_read();
    deliver(
        &mut c,
        meta(9, Phase::Read, 3, 1, clipboard::hash(b"abc")),
        3,
    );
    c.poll(4, |_| Admission::Rejected);
    assert_eq!(c.view(), View::Uncertain);
    assert!(c.packet_erased());
    assert_eq!(c.poll(5, |_| panic!("replay")), 0);
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    c.publish(1, SOURCE, b"x", 1);
    c.poll(2, |_| Admission::Rejected);
    assert_eq!(c.view(), View::Denied);
    assert!(c.storage_erased());
    assert!(send(&mut c, 3).is_empty());
}
#[test]
fn malformed_receipt_after_completed_finish_does_not_revoke_owned_handoff() {
    let mut c = client();
    let mut p = Peer::new(b"completed");
    paste(&mut c, &mut p, 1);
    assert_eq!(c.bytes(), Some(&b"completed"[..]));
    assert!(!c.receive(SHELL, clipboard::REPLY, [1 | (1 << 63), 0, 0], 3));
    assert_eq!(c.bytes(), Some(&b"completed"[..]));
    c.close();
    assert!(c.storage_erased());
}
#[test]
fn duplicate_prior_phase_cannot_advance_or_replace_current_correlation() {
    let mut c = client();
    intent(&mut c, 1, Action::Copy);
    c.take_event();
    c.publish(1, SOURCE, b"0123456789abcdefghijklmnop", 1);
    send(&mut c, 2);
    assert!(deliver(&mut c, meta(1, Phase::Begin, 26, 0, 0), 3));
    assert!(!deliver(&mut c, meta(1, Phase::Begin, 26, 0, 0), 4));
    assert_eq!(c.view(), View::Pending);
    let requests = send(&mut c, 5);
    assert!(matches!(requests[0], Request::Write { offset: 0, .. }));
    assert!(!deliver(&mut c, meta(1, Phase::Begin, 26, 0, 0), 6));
    assert!(send(&mut c, 7).is_empty());
}
#[test]
fn uncertain_can_only_quiesce_through_explicit_cancel_and_its_ack() {
    let mut c = pending_read();
    c.poll(2002, |_| panic!());
    assert_eq!(
        c.take_event(),
        Some(Event::Stopped {
            ticket: 9,
            view: View::Uncertain
        })
    );
    assert!(!deliver(
        &mut c,
        Response::Intent {
            ticket: 10,
            action: Action::Copy
        },
        2003
    ));
    c.cancel(2004);
    assert_eq!(send(&mut c, 2005), vec![Request::Cancel { ticket: 9 }]);
    assert!(!deliver(&mut c, meta(9, Phase::Read, 4, 3, 4), 2006));
    deliver(&mut c, meta(9, Phase::Cancel, 0, 0, 0), 2007);
    assert_eq!(
        c.take_event(),
        Some(Event::Stopped {
            ticket: 9,
            view: View::Cancelled
        })
    );
    intent(&mut c, 11, Action::Copy);
    assert!(matches!(
        c.take_event(),
        Some(Event::Export { ticket: 11, .. })
    ));
}
fn actual_turn(b: &mut broker::Broker, c: &mut Client, subject: Subject, now: u64) {
    c.poll(now, |p| {
        b.receive(
            subject,
            Request::decode(p.opcode, p.args).unwrap(),
            true,
            now,
        )
        .unwrap();
        Admission::Accepted
    });
    let mut responses = vec![];
    b.poll(now, 4, |participant, p| {
        assert_eq!(participant.subject, subject);
        responses.push(p);
        broker::Admission::Accepted
    });
    for p in responses {
        assert!(c.receive(SHELL, p.opcode, p.args, now));
    }
}
#[test]
fn actual_broker_and_client_whole_12k_handoff_and_decline() {
    let mut b = broker::Broker::new();
    let participant = b
        .register(app_lifecycle::CatalogInfo {
            target_pd: 9,
            task_id: 701,
            surface_id: 156,
            coordinator_pd: 3,
        })
        .unwrap();
    let mut c = Client::new();
    assert!(c.bind(participant));
    let ticket = b.issue(participant.subject, Action::Copy, true, 1).unwrap();
    actual_turn(&mut b, &mut c, participant.subject, 1);
    assert_eq!(c.take_event(), Some(Event::Export { ticket, cut: false }));
    let body = vec![b'A'; MAX_BYTES];
    assert!(c.publish(ticket, SOURCE, &body, 1));
    for now in 2..1000 {
        actual_turn(&mut b, &mut c, participant.subject, now);
        if c.view() == View::Copied {
            break;
        }
    }
    assert_eq!(b.publication().unwrap().len, MAX_BYTES);
    assert!(matches!(
        c.take_event(),
        Some(Event::Published { len: MAX_BYTES, .. })
    ));
    assert!(c.storage_erased());
    let ticket = b
        .issue(participant.subject, Action::Paste, true, 1000)
        .unwrap();
    actual_turn(&mut b, &mut c, participant.subject, 1000);
    assert_eq!(c.take_event(), Some(Event::Paste { ticket }));
    assert!(c.read(ticket, 1000));
    for now in 1001..3000 {
        actual_turn(&mut b, &mut c, participant.subject, now);
        if c.bytes().is_some() {
            break;
        }
    }
    assert_eq!(c.bytes(), Some(body.as_slice()));
    assert!(!b.busy());
    assert!(matches!(
        c.take_event(),
        Some(Event::Ready { len: MAX_BYTES, .. })
    ));
    assert!(c.settle(true));
    let ticket = b
        .issue(participant.subject, Action::Copy, true, 3000)
        .unwrap();
    actual_turn(&mut b, &mut c, participant.subject, 3000);
    c.take_event();
    assert!(!c.publish(ticket, SOURCE, b"", 3000));
    actual_turn(&mut b, &mut c, participant.subject, 3001);
    assert_eq!(c.view(), View::Empty);
    assert!(!b.busy());
    assert!(c.storage_erased());
}
#[test]
fn actual_broker_producer_retirement_before_finish_invalidates_client() {
    let mut b = broker::Broker::new();
    let producer = b
        .register(app_lifecycle::CatalogInfo {
            target_pd: 15,
            task_id: 901,
            surface_id: 300,
            coordinator_pd: 3,
        })
        .unwrap();
    let consumer = b
        .register(app_lifecycle::CatalogInfo {
            target_pd: 9,
            task_id: 701,
            surface_id: 156,
            coordinator_pd: 3,
        })
        .unwrap();
    let mut source = Client::new();
    source.bind(producer);
    let mut dest = Client::new();
    dest.bind(consumer);
    let ticket = b.issue(producer.subject, Action::Copy, true, 1).unwrap();
    actual_turn(&mut b, &mut source, producer.subject, 1);
    source.take_event();
    source.publish(ticket, SOURCE, b"private handoff", 1);
    for now in 2..20 {
        actual_turn(&mut b, &mut source, producer.subject, now);
        if source.view() == View::Copied {
            break;
        }
    }
    source.take_event();
    let ticket = b.issue(consumer.subject, Action::Paste, true, 20).unwrap();
    actual_turn(&mut b, &mut dest, consumer.subject, 20);
    dest.take_event();
    dest.read(ticket, 20);
    actual_turn(&mut b, &mut dest, consumer.subject, 21);
    actual_turn(&mut b, &mut dest, consumer.subject, 22);
    assert!(dest.bytes().is_none());
    assert!(b.retire(producer.subject));
    let mut terminal = vec![];
    b.poll(23, 4, |_, p| { terminal.push(p); broker::Admission::Accepted });
    for p in terminal { assert!(dest.receive(SHELL,p.opcode,p.args,23)); }
    assert_eq!(dest.view(), View::Uncertain);
    assert!(dest.storage_erased());
    assert!(
        matches!(dest.take_event(),Some(Event::Stopped{ticket:t,view:View::Uncertain})if t==ticket)
    );
}

#[test]
fn utf8_copy_cut_paste_keeps_exact_bytes_across_every_register_boundary() {
    for scalar in ["é", "€", "𐍈"] { for split in 1..scalar.len() {
        let mut body=vec![b'a';MAX_BYTES];for boundary in (16..MAX_BYTES).step_by(16) {let start=boundary-split;body[start..start+scalar.len()].copy_from_slice(scalar.as_bytes());}
        assert!(clipboard::text(&body));assert!(!clipboard::text(&body[..16]));
        let mut source=client();let mut peer=Peer::new(&[]);
        assert_eq!(copy(&mut source,&mut peer,&body,1,true),Event::Published{ticket:1,cut:true,len:MAX_BYTES,generation:peer.generation,hash:clipboard::hash(&body)});
        assert_eq!(peer.body,body);assert!(source.storage_erased());
        let mut consumer=client();assert_eq!(paste(&mut consumer,&mut peer,2),Event::Ready{ticket:2,len:MAX_BYTES,generation:peer.generation,hash:clipboard::hash(&body)});
        assert_eq!(consumer.bytes(),Some(body.as_slice()));assert!(consumer.finish_read());assert!(consumer.storage_erased());
    }}
}

#[test]
fn malformed_complete_reads_never_send_finish_or_expose_ready_even_with_matching_hash() {
    let mut bodies=vec![vec![0xff],vec![0xc0,0xaf],vec![0xe2,0x82],vec![0xed,0xa0,0x80],vec![0xf4,0x90,0x80,0x80]];
    for scalar in (0..=159).filter(|c|(*c<32&&*c!=9&&*c!=10)||(127..=159).contains(c)) {let mut bytes=[0;4];bodies.push(char::from_u32(scalar).unwrap().encode_utf8(&mut bytes).as_bytes().to_vec());}
    for body in bodies {
        let mut c=client();let mut peer=Peer::new(&[]);peer.body=body.clone();peer.hash=clipboard::hash(&body);intent(&mut c,1,Action::Paste);assert_eq!(c.take_event(),Some(Event::Paste{ticket:1}));assert!(c.read(1,1));
        for now in 2..20 {peer.turn(&mut c,now);assert!(c.bytes().is_none());assert!(!matches!(c.take_event(),Some(Event::Ready{..})));}
        assert_eq!(c.view(),View::Uncertain);assert!(c.storage_erased());assert!(!peer.log.iter().any(|r|matches!(r,Request::Finish{..})));
        let mut exporter=client();intent(&mut exporter,1,Action::Cut);exporter.take_event();assert!(!exporter.publish(1,SOURCE,&body,1));assert!(exporter.storage_erased());assert_eq!(send(&mut exporter,2),vec![Request::Cancel{ticket:1}]);assert!(!matches!(exporter.take_event(),Some(Event::Published{..})));
    }
}

#[test]
fn utf8_tab_and_lf_are_preserved_without_ascii_sanitizing_or_normalization() {
    let body="café\t€\n𐍈".as_bytes();let mut c=client();let mut peer=Peer::new(&[]);
    assert!(matches!(copy(&mut c,&mut peer,body,1,false),Event::Published{cut:false,len,..}if len==body.len()));
    let mut recipient=client();assert!(matches!(paste(&mut recipient,&mut peer,2),Event::Ready{len,..}if len==body.len()));
    assert_eq!(recipient.bytes(),Some(body));assert_eq!(peer.hash,clipboard::hash(body));assert!(recipient.settle(true));assert!(recipient.storage_erased());
}
