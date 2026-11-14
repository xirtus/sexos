// SPDX-License-Identifier: GPL-3.0-only
//! Production wire, Shell broker and SDK compiled unchanged with ONLY native
//! catalog/route/inbox transport mocked. Run --test-threads=1 (one native loop).
#![allow(dead_code)]
extern crate alloc;
extern crate self as sex_pdx;
#[path = "../../../crates/sex-pdx/src/autofill.rs"]
pub mod autofill;
#[path = "../../../servers/silk-shell/src/autofill.rs"]
pub mod broker;
use broker as autofill_broker;
#[path = "../../../servers/silk-shell/src/autofill_routes.rs"]
mod native_routes;
#[path = "../../../crates/sex-app/src/autofill.rs"]
mod sdk;
use autofill::{Packet, Subject};
use broker::{Admission, Broker, Endpoint};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};
#[derive(Clone, Copy)]
pub struct PdxMessage {
    pub type_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
}
pub struct Window {
    pub target: u32,
    pub task: u32,
}
pub const SLOT_SHELL: u64 = 6;
const P: Subject = Subject { pd: 42, task: 100 };
const C: Subject = Subject { pd: 43, task: 101 };
const X: Subject = Subject { pd: 44, task: 102 };
fn endpoint(subject: Subject, provider: bool) -> Endpoint {
    Endpoint {
        subject,
        route: subject.pd as u64,
        name: if provider {
            *b"chastity\0\0\0\0\0\0\0\0"
        } else {
            *b"client\0\0\0\0\0\0\0\0\0\0"
        },
    }
}
struct Env {
    broker: Broker,
    current: Subject,
    focus: Subject,
    now: u64,
    live: Vec<Subject>,
    inbox: Vec<(Subject, VecDeque<PdxMessage>)>,
    admission_failures: usize,
    native: bool,
    contended: bool,
    events: VecDeque<PdxMessage>,
}
static ENV: OnceLock<Mutex<Env>> = OnceLock::new();
fn env<R>(f: impl FnOnce(&mut Env) -> R) -> R {
    f(&mut ENV.get_or_init(|| Mutex::new(fresh())).lock().unwrap())
}
fn fresh() -> Env {
    let mut b = Broker::new();
    for s in [P, C, X] {
        b.bind(endpoint(s, s == P));
    }
    Env {
        broker: b,
        current: C,
        focus: C,
        now: 0,
        live: vec![P, C, X],
        admission_failures: 0,
        native: false,
        contended: false,
        events: VecDeque::new(),
        inbox: vec![
            (P, VecDeque::new()),
            (C, VecDeque::new()),
            (X, VecDeque::new()),
        ],
    }
}
fn reset() {
    env(|e| *e = fresh());
}
fn current(s: Subject) {
    env(|e| e.current = s);
}
pub fn pdx_call(slot: u64, op: u64, a: u64, b: u64, c: u64) -> (u64, u64) {
    let (native, s, focused, now, fail) = env(|e| {
        let fail = e.admission_failures > 0 && slot == SLOT_SHELL;
        if fail {
            e.admission_failures -= 1;
        }
        (e.native, e.current, e.focus == e.current, e.now, fail)
    });
    if fail {
        return (u64::MAX, 0);
    }
    if native {
        if slot == SLOT_SHELL {
            let mut m = PdxMessage {
                type_id: op,
                arg0: a,
                arg1: b,
                arg2: c,
                caller_pd: s.pd,
                caller_task: s.task,
            };
            unsafe {
                native_routes::receive(&mut m);
            }
        } else if slot == 256 {
            let subject = Subject::decode(a).unwrap();
            let name = endpoint(subject, subject == P).name;
            let mut bytes = [0; 3];
            for (i, out) in bytes.iter_mut().enumerate() {
                if let Some(v) = name.get(b as usize * 3 + i) {
                    *out = *v;
                }
            }
            let word = autofill::identity_reply(subject.task, b as usize, bytes);
            env(|e| {
                e.events.push_back(PdxMessage {
                    type_id: 1,
                    arg0: word,
                    arg1: 0,
                    arg2: 0,
                    caller_pd: 11,
                    caller_task: 11,
                })
            });
        } else {
            env(|e| {
                if let Some((_, q)) = e.inbox.iter_mut().find(|(s, _)| s.pd as u64 == slot) {
                    q.push_back(PdxMessage {
                        type_id: op,
                        arg0: a,
                        arg1: b,
                        arg2: c,
                        caller_pd: 3,
                        caller_task: 3,
                    });
                }
            });
        }
    } else {
        env(|e| {
            e.broker.receive(s, &Packet::new(op, a, b, c), focused, now);
        });
    }
    (0, 0)
}
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX;
pub fn get_ticks() -> u64 {
    env(|e| e.now)
}
pub mod program_store {
    pub const SHELL_ROUTE_SLOT: u64 = 256;
}
pub mod clipboard {
    pub use crate::autofill::Subject;
}
fn clipboard_live(s: Subject) -> Option<bool> {
    env(|e| {
        if e.contended {
            None
        } else {
            Some(e.live.contains(&s))
        }
    })
}
struct Participant {
    subject: Subject,
}
fn focused_program_participant() -> Option<Participant> {
    Some(Participant {
        subject: env(|e| e.focus),
    })
}
pub mod app_catalog {
    #[derive(PartialEq, Eq)]
    pub enum App {
        Program,
    }
    pub struct Row {
        pub app: App,
        pub lifecycle_slot: usize,
        pub close_slot: u64,
        pub surface: u64,
    }
    static ROWS: [Row; 3] = [
        Row {
            app: App::Program,
            lifecycle_slot: 42,
            close_slot: 42,
            surface: 42,
        },
        Row {
            app: App::Program,
            lifecycle_slot: 43,
            close_slot: 43,
            surface: 43,
        },
        Row {
            app: App::Program,
            lifecycle_slot: 44,
            close_slot: 44,
            surface: 44,
        },
    ];
    pub fn by_target(pd: u32) -> Option<&'static Row> {
        ROWS.iter().find(|r| r.lifecycle_slot == pd as usize)
    }
}
pub mod app_lifecycle {
    use super::*;
    pub const LIVE: u64 = 5;
    pub struct Catalog {
        subject: Subject,
    }
    impl Catalog {
        pub fn is_sender(&self, pd: u32, task: u32) -> bool {
            self.subject == Subject { pd, task }
        }
        pub fn is_subject(&self, pd: u32, surface: u64, coordinator: u32) -> bool {
            self.subject.pd == pd && pd as u64 == surface && coordinator == 3
        }
    }
    pub fn catalog(slot: usize) -> Result<Catalog, u64> {
        env(|e| {
            e.live
                .iter()
                .find(|s| s.pd as usize == slot)
                .copied()
                .map(|subject| Catalog { subject })
                .ok_or(3)
        })
    }
    pub fn info(_: usize) -> Result<(), u64> {
        Err(LIVE)
    }
}
fn native_pump() {
    for _ in 0..32 {
        unsafe {
            native_routes::poll();
        }
        let event = env(|e| e.events.pop_front());
        if let Some(mut m) = event {
            unsafe {
                native_routes::receive(&mut m);
            }
        } else if !unsafe { native_routes::pending() } {
            break;
        }
    }
}
pub fn pdx_try_listen_raw(_: u64) -> Option<PdxMessage> {
    env(|e| {
        e.inbox
            .iter_mut()
            .find(|(s, _)| *s == e.current)
            .unwrap()
            .1
            .pop_front()
    })
}
pub mod mailbox {
    use super::*;
    pub fn put(m: PdxMessage) {
        env(|e| {
            e.inbox
                .iter_mut()
                .find(|(s, _)| *s == e.current)
                .unwrap()
                .1
                .push_back(m)
        });
    }
    pub fn take(wanted: impl Fn(&PdxMessage) -> bool) -> Option<PdxMessage> {
        env(|e| {
            let queue = &mut e.inbox.iter_mut().find(|(s, _)| *s == e.current).unwrap().1;
            let i = queue.iter().position(wanted)?;
            queue.remove(i)
        })
    }
}
fn pump() {
    for _ in 0..32 {
        let pending = env(|e| {
            let live = e.live.clone();
            let mut messages = Vec::new();
            e.broker.poll(
                e.now,
                |s| Some(live.contains(&s)),
                |target, packet| {
                    messages.push((
                        target.subject,
                        PdxMessage {
                            type_id: packet.opcode,
                            arg0: packet.args[0],
                            arg1: packet.args[1],
                            arg2: packet.args[2],
                            caller_pd: 3,
                            caller_task: 3,
                        },
                    ));
                    Admission::Accepted
                },
            );
            for (s, m) in messages {
                if let Some((_, q)) = e.inbox.iter_mut().find(|(who, _)| *who == s) {
                    q.push_back(m);
                }
            }
            e.broker.pending()
        });
        if !pending {
            break;
        }
    }
}
fn setup() -> (sdk::Provider, sdk::Client) {
    reset();
    current(P);
    let mut provider = sdk::Provider::new(
        &Window {
            target: P.pd,
            task: P.task,
        },
        99,
    )
    .unwrap();
    provider.poll(0);
    pump();
    provider.poll(0);
    assert!(provider.registered());
    current(C);
    let client = sdk::Client::request(
        &Window {
            target: C.pd,
            task: C.task,
        },
        "https://example.org",
        0,
    )
    .unwrap();
    (provider, client)
}
fn step(
    provider: &mut sdk::Provider,
    client: &mut sdk::Client,
    now: u64,
) -> Option<Result<sdk::Credentials, sdk::Error>> {
    env(|e| e.now = now);
    current(P);
    provider.poll(now);
    pump();
    current(C);
    let result = client.poll(now);
    pump();
    result
}
fn offered(provider: &mut sdk::Provider, client: &mut sdk::Client) {
    for tick in 0..100 {
        assert!(step(provider, client, tick).is_none());
        if provider.offer().is_some() {
            return;
        }
    }
    panic!("no offer");
}
#[test]
fn real_sdk_large_transfer_only_after_consent_and_within_deadline() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    for tick in 100..200 {
        assert!(step(&mut p, &mut c, tick).is_none());
    }
    let offer = p.offer().unwrap();
    assert_eq!(offer.consumer, C);
    assert_eq!(offer.origin, "https://example.org");
    env(|e| e.focus = P);
    let user = "u".repeat(4096);
    let pass = "p".repeat(4096);
    p.allow(&user, &pass, 200).unwrap();
    for tick in 200..10000 {
        if let Some(result) = step(&mut p, &mut c, tick) {
            let got = result.unwrap();
            got.with(|u, p| {
                assert_eq!(u, user);
                assert_eq!(p, pass);
            });
            assert!(tick < autofill::TRANSFER_TICKS);
            return;
        }
    }
    panic!("large transfer stalled");
}
#[test]
fn deny_never_transfers_credentials() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    current(P);
    p.deny();
    pump();
    current(C);
    assert!(matches!(c.poll(200), Some(Err(sdk::Error::Declined))));
}
#[test]
fn lock_revokes_a_pending_request() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    current(P);
    p.close();
    p.poll(200);
    pump();
    current(P);
    p.poll(200);
    assert!(p.revoked());
    current(C);
    assert!(matches!(c.poll(200), Some(Err(sdk::Error::Locked))));
}
#[test]
fn client_must_be_focused_to_begin() {
    let (mut p, mut c) = setup();
    env(|e| e.focus = P);
    for tick in 0..20 {
        if let Some(r) = step(&mut p, &mut c, tick) {
            assert!(matches!(r, Err(sdk::Error::Denied)));
            assert!(p.offer().is_none());
            return;
        }
    }
    panic!("background request not denied");
}
#[test]
fn provider_must_be_focused_to_approve() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    p.allow("alice", "fixturesecret", 100).unwrap();
    for tick in 100..120 {
        if let Some(r) = step(&mut p, &mut c, tick) {
            assert!(matches!(r, Err(sdk::Error::Denied)));
            return;
        }
    }
    panic!("background approval not denied");
}
#[test]
fn client_cannot_impersonate_provider_or_approve() {
    reset();
    let status = env(|e| {
        e.broker
            .receive(C, &Packet::new(autofill::REGISTER, 8, 0, 0), true, 0)
    });
    assert_eq!(status, autofill::Status::Denied);
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    let token = p.offer().unwrap().token;
    let status = env(|e| {
        e.broker.receive(
            C,
            &Packet::new(autofill::ALLOW, token as u64, 99, 5 | (6 << 16)),
            true,
            100,
        )
    });
    assert_eq!(status, autofill::Status::Denied);
    assert!(step(&mut p, &mut c, 101).is_none());
}
#[test]
fn stale_provider_epoch_and_replayed_chunk_are_refused() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    let token = p.offer().unwrap().token;
    let early = autofill::chunk(autofill::DATA, token, 99, 0, b"secret").unwrap();
    assert_eq!(
        env(|e| e.broker.receive(P, &early, true, 100)),
        autofill::Status::Denied
    );
    env(|e| e.focus = P);
    p.allow("alice", "fixturesecret", 100).unwrap();
    for tick in 100..200 {
        if step(&mut p, &mut c, tick).is_some() {
            break;
        }
    }
    assert_eq!(
        env(|e| e.broker.receive(P, &early, true, 201)),
        autofill::Status::Stale
    );
    let wrong = autofill::chunk(autofill::DATA, token, 98, 0, b"secret").unwrap();
    assert_eq!(
        env(|e| e.broker.receive(P, &wrong, true, 202)),
        autofill::Status::Stale
    );
}
#[test]
fn exact_run_reuse_revokes_old_outbox_before_any_new_delivery() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    let token = p.offer().unwrap().token;
    env(|e| {
        e.live.retain(|s| *s != C);
        let new = Subject {
            pd: C.pd,
            task: C.task + 1,
        };
        e.broker.bind(endpoint(new, false));
        e.live.push(new);
        e.inbox.push((new, VecDeque::new()));
    });
    env(|e| e.focus = P);
    p.allow("alice", "fixturesecret", 100).unwrap();
    for tick in 100..130 {
        step(&mut p, &mut c, tick);
    }
    let old = autofill::chunk(autofill::DATA, token, 99, 0, b"secret").unwrap();
    assert_ne!(
        env(|e| e.broker.receive(P, &old, true, 200)),
        autofill::Status::Ok
    );
    assert!(env(|e| e
        .inbox
        .last()
        .unwrap()
        .1
        .iter()
        .all(|m| m.type_id != autofill::DATA)));
}
#[test]
fn malformed_chunk_padding_bounds_and_epoch_refused() {
    assert!(autofill::chunk(autofill::DATA, 1, 0, 0, b"secret").is_none());
    let mut packet = autofill::chunk(autofill::DATA, 1, 9, 0, b"x").unwrap();
    packet.args[2] |= 1 << 56;
    assert!(autofill::chunk_meta(&packet).is_none());
    assert!(autofill::lengths(4097 | 2 << 16).is_none());
    assert!(autofill::lengths(1).is_none());
}
#[test]
fn clock_regression_and_expiry_erase_pending_consent() {
    for expired in [0, autofill::CONSENT_TICKS + 1] {
        let (mut p, mut c) = setup();
        offered(&mut p, &mut c);
        env(|e| e.now = 100);
        env(|e| {
            e.broker
                .poll(expired, |_| Some(true), |_, _| Admission::Accepted)
        });
        for tick in 0..20 {
            if let Some(r) = step(&mut p, &mut c, autofill::CONSENT_TICKS + 2 + tick) {
                assert!(r.is_err());
                break;
            }
        }
        assert!(p.offer().is_none());
    }
}
#[test]
fn foreign_status_flood_cannot_consume_active_transfer_capacity() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    env(|e| e.focus = P);
    p.allow(&"u".repeat(4096), &"p".repeat(4096), 100).unwrap();
    for tick in 100..10000 {
        env(|e| {
            for seq in 1..32 {
                e.broker
                    .receive(X, &Packet::new(autofill::BEGIN, seq, 10, 0), false, tick);
            }
        });
        if let Some(result) = step(&mut p, &mut c, tick) {
            let credentials = result.unwrap();
            credentials.with(|u, p| {
                assert_eq!(u.len(), 4096);
                assert_eq!(p.len(), 4096);
            });
            return;
        }
    }
    panic!("foreign traffic stalled valid transfer");
}
#[test]
fn origins_reject_multiline_and_unicode_bidi() {
    for bad in [
        "example.org\nAllow",
        "example.org\rfoo",
        "example.org\u{202e}moc",
        "example.org\u{2066}evil",
        "example.org with space",
    ] {
        assert!(!autofill::valid_origin(bad.as_bytes()));
        let window = Window {
            target: C.pd,
            task: C.task,
        };
        assert!(sdk::Client::request(&window, bad, 0).is_err());
    }
    assert!(autofill::valid_origin(b"https://xn--bcher-kva.example/dav"));
}
#[test]
fn identity_reply_binds_full_run_task_and_part() {
    for part in 0..6 {
        let word = autofill::identity_reply(0xf1234567, part, *b"abc");
        assert_eq!(
            autofill::identity_decode(word),
            Some((0xf1234567, part, *b"abc"))
        );
    }
    assert!(autofill::identity_decode(0).is_none());
}
#[test]
fn cancellation_before_ready_uses_exact_client_sequence() {
    let (mut p, mut c) = setup();
    current(C);
    assert!(c.poll(0).is_none());
    c.cancel();
    c.poll(1);
    pump();
    current(C);
    let _ = c.poll(1);
    current(P);
    p.poll(1);
    assert!(p.offer().is_none());
    current(C);
    let mut next = sdk::Client::request(
        &Window {
            target: C.pd,
            task: C.task,
        },
        "https://example.org",
        2,
    )
    .unwrap();
    for tick in 2..100 {
        let _ = step(&mut p, &mut next, tick);
        if p.offer().is_some() {
            return;
        }
    }
    panic!("cancelled early request still blocks next client");
}

#[test]
fn provider_close_retries_kernel_full_before_reporting_locked() {
    let (mut p, mut c) = setup();
    offered(&mut p, &mut c);
    env(|e| e.admission_failures = 3);
    current(P);
    p.close();
    assert!(p.offer().is_none());
    assert!(!p.revoked());
    for tick in 200..203 {
        p.poll(tick);
        assert!(!p.revoked());
        assert!(!p.registered());
    }
    p.poll(203);
    pump();
    p.poll(204);
    assert!(p.revoked());
    current(C);
    assert!(matches!(c.poll(204), Some(Err(sdk::Error::Locked))));
}
#[cfg(test)]
fn native_revoke_under_contention(retirement: bool) {
    reset();
    env(|e| e.native = true);
    unsafe {
        native_routes::reset_for_host_test();
        for s in [P, C, X] {
            native_routes::bound(s.pd, s.task);
        }
    }
    native_pump();
    current(P);
    let mut p = sdk::Provider::new(
        &Window {
            target: P.pd,
            task: P.task,
        },
        99,
    )
    .unwrap();
    p.poll(0);
    native_pump();
    p.poll(1);
    assert!(p.registered());
    current(C);
    let mut c = sdk::Client::request(
        &Window {
            target: C.pd,
            task: C.task,
        },
        "https://example.org",
        0,
    )
    .unwrap();
    for tick in 1..200 {
        env(|e| e.now = tick);
        current(P);
        p.poll(tick);
        native_pump();
        current(C);
        assert!(c.poll(tick).is_none());
        native_pump();
        if p.offer().is_some() {
            break;
        }
    }
    assert!(p.offer().is_some());
    env(|e| e.focus = P);
    current(P);
    p.allow("alice", "a secret that must be revoked", 200)
        .unwrap();
    // The approved header is delivered and acknowledged before pausing custody.
    for tick in 200..204 {
        current(P);
        p.poll(tick);
        native_pump();
        current(C);
        assert!(c.poll(tick).is_none());
        native_pump();
    }
    env(|e| {
        e.contended = true;
    });
    current(X);
    for n in 0..8 {
        pdx_call(SLOT_SHELL, autofill::BEGIN, 900 + n, 3, 0);
    }
    env(|e| e.admission_failures = 3);
    current(P);
    p.close();
    for tick in 204..208 {
        p.poll(tick);
        assert!(!p.revoked());
    }
    if retirement {
        env(|e| e.live.retain(|s| *s != P));
    }
    // Even though LIVE is unavailable and the native intake is saturated, the
    // exact authenticated revocation bypasses it and erases pending DATA.
    env(|e| e.contended = false);
    native_pump();
    p.poll(208);
    native_pump();
    p.poll(209);
    if retirement {
        // Close tombstones custody before the app receives its close event. The
        // dead run cannot receive an ACK, but actual retirement is the fence.
        unsafe {
            native_routes::retire(P);
        }
        native_pump();
        assert!(!p.revoked());
        env(|e| {
            assert!(e
                .inbox
                .iter()
                .find(|(s, _)| *s == C)
                .unwrap()
                .1
                .iter()
                .all(|m| m.type_id != autofill::DATA))
        });
        current(C);
        assert!(!matches!(c.poll(209), Some(Ok(_))));
    } else {
        assert!(p.revoked());
        current(C);
        assert!(matches!(c.poll(209), Some(Err(sdk::Error::Locked))));
    }
}
#[test]
fn native_full_intake_and_live_contention_cannot_drop_lock_revocation() {
    native_revoke_under_contention(false);
}
#[test]
fn native_close_tombstone_and_retirement_erase_queued_data_at_full_intake() {
    native_revoke_under_contention(true);
}
