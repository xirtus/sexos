//! Host tests of IPC v2 one-shot reply capabilities (kernel/src/ipc_v2/reply.rs).
#[path = "../kernel/src/ipc_v2"]
mod ipc_v2 {
    pub type TaskId = u64;
    pub mod endpoint;
    pub mod reply;
}

use ipc_v2::endpoint::{CallKind, Sender};
use ipc_v2::reply::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Stand-in for the kernel generation registry: one word per task.
struct Gens([AtomicU32; 8]);
impl Gens {
    fn new() -> Self { Self(Default::default()) }
    fn get(&self, t: u64) -> u32 { self.0[t as usize].load(Ordering::SeqCst) }
    fn retire(&self, t: u64) { self.0[t as usize].fetch_add(1, Ordering::SeqCst); }
}
impl Incarnations for Gens {
    fn is_current(&self, t: u64, g: u32) -> bool { self.0.get(t as usize).is_some_and(|w| w.load(Ordering::SeqCst) == g) }
}

fn caller(gens: &Gens, task: u64) -> Sender { Sender { task, badge: 1, generation: gens.get(task), kind: CallKind::Call } }

#[test]
fn reply_is_used_at_most_once() {
    let gens = Gens::new();
    let slot = ReplySlot::new();
    slot.install(ReplyCap::for_sender(&caller(&gens, 3)).unwrap()).unwrap();
    assert!(slot.is_live(&gens));
    assert_eq!(slot.take(&gens), Ok(ReplyCap { caller: 3, generation: 0 }));
    assert_eq!(slot.take(&gens), Err(ReplyError::Empty));
}

#[test]
fn one_way_sends_create_no_reply() {
    let s = Sender { task: 1, badge: 0, generation: 0, kind: CallKind::Send };
    assert_eq!(ReplyCap::for_sender(&s), None);
}

#[test]
fn a_thread_holds_one_unanswered_call() {
    let gens = Gens::new();
    let slot = ReplySlot::new();
    slot.install(ReplyCap { caller: 1, generation: 0 }).unwrap();
    assert_eq!(slot.install(ReplyCap { caller: 2, generation: 0 }), Err(ReplyError::Occupied));
    assert_eq!(slot.peek(), Some(ReplyCap { caller: 1, generation: 0 }));
    let _ = gens;
}

#[test]
fn caller_death_cancels_the_outstanding_reply() {
    let gens = Gens::new();
    let slot = ReplySlot::new();
    slot.install(ReplyCap::for_sender(&caller(&gens, 5)).unwrap()).unwrap();
    gens.retire(5);
    assert!(!slot.is_live(&gens), "deferred work sees the caller is gone without a syscall");
    assert_eq!(slot.take(&gens), Err(ReplyError::CallerGone));
    assert_eq!(slot.take(&gens), Err(ReplyError::Empty), "the dead capability is consumed");
    // The task id's next incarnation gets its own replies.
    slot.install(ReplyCap::for_sender(&caller(&gens, 5)).unwrap()).unwrap();
    assert_eq!(slot.take(&gens), Ok(ReplyCap { caller: 5, generation: 1 }));
}

#[test]
fn server_death_returns_a_live_caller_to_wake() {
    let gens = Gens::new();
    let slot = ReplySlot::new();
    slot.install(ReplyCap { caller: 1, generation: 0 }).unwrap();
    assert_eq!(slot.discard(&gens), Some(ReplyCap { caller: 1, generation: 0 }));
    slot.install(ReplyCap { caller: 2, generation: 0 }).unwrap();
    gens.retire(2);
    assert_eq!(slot.discard(&gens), None);
    assert_eq!(slot.peek(), None);
}

#[test]
fn unknown_callers_are_never_current() {
    let gens = Gens::new();
    let slot = ReplySlot::new();
    slot.install(ReplyCap { caller: 4000, generation: 0 }).unwrap();
    assert_eq!(slot.take(&gens), Err(ReplyError::CallerGone));
}
