// SPDX-License-Identifier: GPL-3.0-only
//! Model-based property test of the kernel reply-obligation table (kernel plan phase 7).
//! Random grant/withdraw/consume/holds/clear sequences run against the real
//! kernel/src/ipc/reply_credit.rs and a plain map. Every answer must match.
//!
//! The model writes down one rule the code only implies: a caller keeps its table
//! entry after its count falls to zero, until `clear`. So a callee can be owed by at
//! most REPLY_CALLERS distinct callers per incarnation, ever, not at once. That is
//! safe while the boot registry holds fewer callers than REPLY_CALLERS; a registry
//! bigger than that would refuse new callers with ERR_SERVICE_NOT_READY until the
//! callee restarts.
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_reply_credit
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[path = "../kernel/src/ipc/reply_credit.rs"]
#[allow(dead_code)]
mod reply_credit;
use reply_credit::{Grant, ReplyCredits, REPLY_CALLERS, REPLY_CREDIT_CAPACITY};

#[derive(Clone, Debug)]
enum Op { Grant(u32), Withdraw(u32), Consume(u32), Holds(u32), Clear }

#[derive(Default)]
struct Model { counts: BTreeMap<u32, u64>, entries: BTreeSet<u32> }

impl Model {
    fn grant(&mut self, caller: u32) -> Grant {
        if caller == 0 || caller as usize >= REPLY_CREDIT_CAPACITY { return Grant::Full; }
        if !self.entries.contains(&caller) && self.entries.len() == REPLY_CALLERS { return Grant::Full; }
        self.entries.insert(caller);
        *self.counts.entry(caller).or_default() += 1;
        Grant::Added
    }
    fn consume(&mut self, target: u32) -> bool {
        match self.counts.get_mut(&target) { Some(c) if *c > 0 => { *c -= 1; true } _ => false }
    }
    fn holds(&self, target: u32) -> bool { self.counts.get(&target).is_some_and(|c| *c > 0) }
    fn outstanding(&self) -> u128 { self.counts.values().map(|c| *c as u128).sum() }
}

/// Callers drawn mostly from a small pool (so they collide and repeat), sometimes
/// from a pool wider than the table (so it fills), plus invalid edges.
fn caller(wide: u32) -> impl Strategy<Value = u32> {
    prop_oneof![
        8 => 1u32..24,
        4 => 1u32..wide,
        1 => Just(0u32),
        1 => (REPLY_CREDIT_CAPACITY as u32)..u32::MAX,
        // Same home slot (caller % REPLY_CALLERS) to drive the probe chain.
        2 => (0u32..16).prop_map(|k| 5 + k * REPLY_CALLERS as u32),
    ]
}

fn op(wide: u32) -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => caller(wide).prop_map(Op::Grant),
        2 => caller(wide).prop_map(Op::Withdraw),
        5 => caller(wide).prop_map(Op::Consume),
        3 => caller(wide).prop_map(Op::Holds),
        1 => Just(Op::Clear),
    ]
}

fn run(ops: &[Op]) -> Result<(), TestCaseError> {
    let mut real = Box::new(ReplyCredits::new());
    let mut model = Model::default();
    for (step, op) in ops.iter().enumerate() {
        match *op {
            Op::Grant(c) => prop_assert_eq!(real.grant(c), model.grant(c), "step {} {:?}", step, op),
            Op::Withdraw(c) => { real.withdraw(c); model.consume(c); }
            Op::Consume(c) => prop_assert_eq!(real.consume(c), model.consume(c), "step {} {:?}", step, op),
            Op::Holds(c) => prop_assert_eq!(real.holds(c), model.holds(c), "step {} {:?}", step, op),
            Op::Clear => { real.clear(); model = Model::default(); }
        }
        prop_assert_eq!(real.outstanding(), model.outstanding(), "step {} {:?}", step, op);
    }
    // Draining every caller leaves nothing owed, and nothing more can be spent.
    for (&c, &n) in &model.counts {
        for _ in 0..n { prop_assert!(real.consume(c)); }
        prop_assert!(!real.consume(c) && !real.holds(c));
    }
    prop_assert_eq!(real.outstanding(), 0);
    Ok(())
}

proptest! {
    #[test]
    fn matches_the_map_model_with_few_callers(ops in proptest::collection::vec(op(40), 0..400)) { run(&ops)?; }

    #[test]
    fn matches_the_map_model_when_the_table_fills(ops in proptest::collection::vec(op(600), 0..1500)) { run(&ops)?; }

    /// A reply can never be spent by anyone but a caller with an accepted call.
    #[test]
    fn no_reply_without_a_grant(granted in proptest::collection::vec(caller(600), 0..200), probe in caller(600)) {
        let mut real = Box::new(ReplyCredits::new());
        let mut added = BTreeSet::new();
        for c in granted { if real.grant(c) == Grant::Added { added.insert(c); } }
        if !added.contains(&probe) {
            prop_assert!(!real.holds(probe));
            prop_assert!(!real.consume(probe));
        }
    }
}

/// The finding above, pinned so a change to it is noticed: after REPLY_CALLERS
/// distinct callers have each been answered, a new caller is still refused.
#[test]
fn answered_callers_keep_their_entries_until_clear() {
    let mut real = Box::new(ReplyCredits::new());
    for c in 1..=REPLY_CALLERS as u32 {
        assert_eq!(real.grant(c), Grant::Added);
        assert!(real.consume(c));
    }
    assert_eq!(real.outstanding(), 0);
    assert_eq!(real.grant(REPLY_CALLERS as u32 + 1), Grant::Full, "entries outlive their counts");
    real.clear();
    assert_eq!(real.grant(REPLY_CALLERS as u32 + 1), Grant::Added);
}
