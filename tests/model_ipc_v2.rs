// SPDX-License-Identifier: GPL-3.0-only
//! Model-based tests of the IPC v2 objects (kernel plan phase 7): the real
//! kernel/src/ipc_v2/{endpoint,notification,reply}.rs against plain references.
//!
//! Endpoint: one waiting receiver or a FIFO of at most 16 callers, closed for good
//! until reopened; close hands back everyone waiting. Notification: a word of
//! pending bits and at most one waiter; a signal wakes the waiter exactly when one
//! is parked. Reply: one slot per server thread, used at most once, refused once
//! the caller's incarnation moves on. IRQ routes: one notification bit per line.
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_ipc_v2
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};

use proptest::prelude::*;
use sexos_kernel_models::ipc_v2::endpoint::{CallKind, Endpoint, Released, RecvOutcome, SendOutcome, Sender, QUEUE_DEPTH};
use sexos_kernel_models::ipc_v2::notification::{BindError, IrqRoutes, Notification, SignalOutcome, WaitOutcome};
use sexos_kernel_models::ipc_v2::reply::{Incarnations, ReplyCap, ReplyError, ReplySlot};

fn config() -> ProptestConfig {
    ProptestConfig { cases: std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(256), ..ProptestConfig::default() }
}

// ---------------------------------------------------------------------------
// Endpoint

#[derive(Clone, Debug)]
enum EpOp { Send(u64, u64, bool), Recv(u64), CancelRecv(u64), CancelSend(u64), Close, Reopen }

fn ep_op() -> impl Strategy<Value = EpOp> {
    prop_oneof![
        6 => (1u64..24, 0u64..4, any::<bool>()).prop_map(|(t, b, call)| EpOp::Send(t, b, call)),
        4 => (0u64..3).prop_map(EpOp::Recv),
        1 => (0u64..3).prop_map(EpOp::CancelRecv),
        1 => (1u64..24).prop_map(EpOp::CancelSend),
        1 => Just(EpOp::Close),
        1 => Just(EpOp::Reopen),
    ]
}

#[derive(Default)]
struct EpModel { waiter: Option<u64>, closed: bool, queue: VecDeque<Sender> }

fn endpoint_run(ops: Vec<EpOp>) -> Result<(), TestCaseError> {
    let ep = Endpoint::new();
    let mut m = EpModel::default();
    for op in ops {
        match op {
            EpOp::Send(task, badge, call) => {
                let s = Sender { task, badge, generation: 1, kind: if call { CallKind::Call } else { CallKind::Send } };
                let want = if m.closed { SendOutcome::Closed }
                    else if let Some(r) = m.waiter.take() { SendOutcome::Rendezvous { receiver: r } }
                    else if m.queue.len() == QUEUE_DEPTH { SendOutcome::Full }
                    else { m.queue.push_back(s); SendOutcome::Queued };
                prop_assert_eq!(ep.send(s), want);
            }
            EpOp::Recv(r) => {
                let want = if m.closed { RecvOutcome::Closed }
                    else if let Some(s) = m.queue.pop_front() { RecvOutcome::Message(s) }
                    else if m.waiter == Some(r) { RecvOutcome::Parked }
                    else if m.waiter.is_some() { RecvOutcome::Busy }
                    else { m.waiter = Some(r); RecvOutcome::Parked };
                prop_assert_eq!(ep.recv(r), want);
            }
            EpOp::CancelRecv(r) => {
                let want = !m.closed && m.waiter == Some(r);
                if want { m.waiter = None; }
                prop_assert_eq!(ep.cancel_recv(r), want);
            }
            EpOp::CancelSend(t) => {
                let want = m.queue.iter().position(|s| s.task == t).map(|i| m.queue.remove(i).unwrap());
                prop_assert_eq!(ep.cancel_send(t), want);
            }
            EpOp::Close => {
                let mut want = Vec::new();
                if !m.closed {
                    if let Some(r) = m.waiter.take() { want.push(Released::Receiver(r)); }
                }
                want.extend(m.queue.drain(..).map(Released::Caller));
                m.closed = true;
                let mut got = Vec::new();
                prop_assert_eq!(ep.close(|r| got.push(r)), want.len());
                prop_assert_eq!(got, want);
            }
            EpOp::Reopen => {
                let want = m.closed;
                m.closed = false;
                prop_assert_eq!(ep.reopen(), want);
            }
        }
        // Nobody is left both queued and with a receiver parked: no lost wakeup.
        prop_assert!(m.queue.is_empty() || m.waiter.is_none());
        prop_assert_eq!(ep.queued(), m.queue.len());
        prop_assert_eq!(ep.waiter(), if m.closed { None } else { m.waiter });
        prop_assert_eq!(ep.is_closed(), m.closed);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Notification and IRQ routes

#[derive(Clone, Debug)]
enum NOp { Signal(u64), Wait(u64), Poll, Cancel(u64) }

fn n_op() -> impl Strategy<Value = NOp> {
    prop_oneof![
        4 => prop_oneof![Just(0u64), 1u64..16, any::<u64>()].prop_map(NOp::Signal),
        4 => (0u64..3).prop_map(NOp::Wait),
        1 => Just(NOp::Poll),
        1 => (0u64..3).prop_map(NOp::Cancel),
    ]
}

fn notification_run(ops: Vec<NOp>) -> Result<(), TestCaseError> {
    let n = Notification::new();
    let (mut word, mut waiter) = (0u64, None::<u64>);
    for op in ops {
        match op {
            NOp::Signal(bits) => {
                let want = if bits == 0 { SignalOutcome::Pending } else {
                    word |= bits;
                    match waiter.take() { Some(w) => SignalOutcome::Wake(w), None => SignalOutcome::Pending }
                };
                prop_assert_eq!(n.signal(bits), want);
            }
            NOp::Wait(t) => {
                let want = if word != 0 { WaitOutcome::Ready(std::mem::take(&mut word)) }
                    else if waiter.is_none() || waiter == Some(t) { waiter = Some(t); WaitOutcome::Parked }
                    else { WaitOutcome::Busy };
                prop_assert_eq!(n.wait(t), want);
            }
            NOp::Poll => prop_assert_eq!(n.poll(), std::mem::take(&mut word)),
            NOp::Cancel(t) => {
                let want = waiter == Some(t);
                if want { waiter = None; }
                prop_assert_eq!(n.cancel_wait(t), want);
            }
        }
        // A parked waiter never sits next to pending bits (they would wake it).
        prop_assert!(waiter.is_none() || word == 0);
        prop_assert_eq!(n.pending(), word);
        prop_assert_eq!(n.waiter(), waiter);
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum RouteOp { Bind(usize, u32, u32), Unbind(usize, u32), UnbindAll(u32) }

fn route_op() -> impl Strategy<Value = RouteOp> {
    prop_oneof![
        3 => (0usize..10, prop_oneof![0u32..4, Just(u32::MAX)], 0u32..70).prop_map(|(l, n, b)| RouteOp::Bind(l, n, b)),
        1 => (0usize..10, 0u32..4).prop_map(|(l, n)| RouteOp::Unbind(l, n)),
        1 => (0u32..4).prop_map(RouteOp::UnbindAll),
    ]
}

fn routes_run(ops: Vec<RouteOp>) -> Result<(), TestCaseError> {
    const LINES: usize = 8;
    let routes = IrqRoutes::<LINES>::new();
    let mut m: BTreeMap<usize, (u32, u32)> = BTreeMap::new();
    for op in ops {
        match op {
            RouteOp::Bind(line, n, bit) => {
                let want = if line >= LINES { Err(BindError::BadLine) }
                    else if bit >= 64 || n == u32::MAX { Err(BindError::BadBit) }
                    else if m.contains_key(&line) { Err(BindError::Bound) }
                    else { m.insert(line, (n, bit)); Ok(()) };
                prop_assert_eq!(routes.bind(line, n, bit), want);
            }
            RouteOp::Unbind(line, n) => {
                let want = m.get(&line).is_some_and(|&(bound, _)| bound == n);
                if want { m.remove(&line); }
                prop_assert_eq!(routes.unbind(line, n), want);
            }
            RouteOp::UnbindAll(n) => {
                let lines: Vec<usize> = m.iter().filter(|(_, &(b, _))| b == n).map(|(l, _)| *l).collect();
                for l in &lines { m.remove(l); }
                prop_assert_eq!(routes.unbind_all(n), lines.len());
            }
        }
        for line in 0..LINES + 2 {
            prop_assert_eq!(routes.route(line), m.get(&line).map(|&(n, b)| (n, 1u64 << b)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reply slot

struct Gens(Vec<AtomicU32>);
impl Incarnations for Gens {
    fn is_current(&self, task: u64, generation: u32) -> bool {
        self.0.get(task as usize).is_some_and(|g| g.load(Ordering::SeqCst) == generation)
    }
}

#[derive(Clone, Debug)]
enum ROp { Install(u64), Take, IsLive, Discard, Die(u64) }

fn r_op() -> impl Strategy<Value = ROp> {
    prop_oneof![
        3 => (1u64..6).prop_map(ROp::Install),
        3 => Just(ROp::Take),
        1 => Just(ROp::IsLive),
        1 => Just(ROp::Discard),
        2 => (1u64..6).prop_map(ROp::Die),
    ]
}

fn reply_run(ops: Vec<ROp>) -> Result<(), TestCaseError> {
    let gens = Gens((0..5).map(|_| AtomicU32::new(1)).collect());
    let slot = ReplySlot::new();
    let mut held: Option<ReplyCap> = None;
    let mut answered: Vec<ReplyCap> = Vec::new();
    for op in ops {
        let current = |c: &ReplyCap| gens.is_current(c.caller, c.generation);
        match op {
            ROp::Install(caller) => {
                let gen = gens.0.get(caller as usize).map_or(0, |g| g.load(Ordering::SeqCst));
                let cap = ReplyCap { caller, generation: gen };
                let want = if held.is_some() { Err(ReplyError::Occupied) } else { held = Some(cap); Ok(()) };
                prop_assert_eq!(slot.install(cap), want);
            }
            ROp::Take | ROp::Discard => {
                let want = match held.take() {
                    None => Err(ReplyError::Empty),
                    Some(c) if current(&c) => Ok(c),
                    Some(_) => Err(ReplyError::CallerGone),
                };
                let got = if matches!(op, ROp::Take) { slot.take(&gens) } else { slot.discard(&gens).ok_or(ReplyError::Empty) };
                if matches!(op, ROp::Take) { prop_assert_eq!(got, want); } else { prop_assert_eq!(got.ok(), want.ok()); }
                if let Ok(c) = got {
                    // Used at most once per installed capability, and only for the
                    // incarnation that called.
                    prop_assert!(current(&c));
                    answered.push(c);
                }
            }
            ROp::IsLive => prop_assert_eq!(slot.is_live(&gens), held.is_some_and(|c| current(&c))),
            ROp::Die(task) => { if let Some(g) = gens.0.get(task as usize) { g.fetch_add(1, Ordering::SeqCst); } }
        }
        prop_assert_eq!(slot.peek(), held);
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]
    #[test]
    fn endpoint_matches_reference(ops in prop::collection::vec(ep_op(), 1..120)) { endpoint_run(ops)?; }
    #[test]
    fn notification_matches_reference(ops in prop::collection::vec(n_op(), 1..80)) { notification_run(ops)?; }
    #[test]
    fn irq_routes_match_reference(ops in prop::collection::vec(route_op(), 1..60)) { routes_run(ops)?; }
    #[test]
    fn reply_slot_matches_reference(ops in prop::collection::vec(r_op(), 1..60)) { reply_run(ops)?; }
}
