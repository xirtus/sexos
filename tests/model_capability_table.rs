// SPDX-License-Identifier: GPL-3.0-only
//! Model-based test of the authority v2 capability table (kernel plan phase 7).
//!
//! Random sequences of create, derive, install, slot revoke, object revoke, destroy,
//! reply install and incarnation end run against the real kernel/src/capability_table.rs
//! and a plain reference: a map of live objects (owner, generation), each domain's
//! table as a map slot -> capability, and each domain's incarnation. After every step
//! every held capability must read back exactly, be honoured exactly when the
//! reference says it is live, and every derivation must match the reference's answer
//! (so authority is only ever narrowed, a badge never changes, and nothing dead
//! comes back).
//! Run: cargo test --manifest-path fuzz/models/Cargo.toml --test model_capability_table
use std::collections::BTreeMap;

use proptest::prelude::*;
use sexos_kernel_models::capability::CapabilityData as D;
use sexos_kernel_models::capability_table::{next_generation, rights, CapabilityTable, Generations, MintError, ObjectCap};

const DOMAINS: u32 = 3;

#[derive(Clone, Debug)]
enum Op {
    Create { owner: u32, notification: bool },
    Derive { from: u32, pick: usize, rights: u32, badge: u64, to: u32 },
    InstallReply { holder: u32, caller: u32 },
    RevokeSlot { domain: u32, pick: usize },
    RevokeObject { pick: usize },
    Destroy { pick: usize },
    EndIncarnation { domain: u32 },
}

fn op() -> impl Strategy<Value = Op> {
    let domain = 0..DOMAINS;
    prop_oneof![
        3 => (domain.clone(), any::<bool>()).prop_map(|(owner, notification)| Op::Create { owner, notification }),
        6 => (domain.clone(), any::<usize>(), 0u32..32, prop_oneof![Just(0u64), 1u64..4], domain.clone())
            .prop_map(|(from, pick, rights, badge, to)| Op::Derive { from, pick, rights, badge, to }),
        1 => (domain.clone(), domain.clone()).prop_map(|(holder, caller)| Op::InstallReply { holder, caller }),
        2 => (domain.clone(), any::<usize>()).prop_map(|(domain, pick)| Op::RevokeSlot { domain, pick }),
        1 => any::<usize>().prop_map(|pick| Op::RevokeObject { pick }),
        1 => any::<usize>().prop_map(|pick| Op::Destroy { pick }),
        1 => domain.prop_map(|domain| Op::EndIncarnation { domain }),
    ]
}

/// Domains are pd ids 1..=DOMAINS (pd 0 is never an owner here).
fn pd(domain: u32) -> u32 { domain + 1 }

#[derive(Default)]
struct Model {
    /// object -> (owner pd, live generation)
    objects: BTreeMap<u32, (u32, u32)>,
    /// object -> generation it will have when next created (survives destroy)
    generation: BTreeMap<u32, u32>,
    tables: Vec<BTreeMap<u32, D>>,
    incarnation: BTreeMap<u32, u32>,
}

impl Model {
    fn generation_of(&self, object: u32) -> u32 { *self.generation.get(&object).unwrap_or(&1) }
    fn incarnation_of(&self, pd: u32) -> u32 { *self.incarnation.get(&pd).unwrap_or(&1) }
    fn live(&self, cap: &ObjectCap) -> bool {
        self.objects.get(&cap.object).is_some_and(|&(_, g)| g == cap.generation)
    }
    fn first_free(&self, domain: usize) -> u32 {
        (1..).find(|id| !self.tables[domain].contains_key(id)).unwrap()
    }
    fn destroy(&mut self, object: u32) {
        self.objects.remove(&object);
        let g = next_generation(self.generation_of(object));
        self.generation.insert(object, g);
    }
}

fn cap_of(d: &D) -> ObjectCap {
    match *d { D::Endpoint(c) | D::Notification(c) | D::Reply(c) => c, D::Domain(_) => unreachable!() }
}

fn run(ops: Vec<Op>) -> Result<(), TestCaseError> {
    let objects: &'static Generations = Box::leak(Box::new(Generations::new()));
    let tables: Vec<Box<CapabilityTable>> =
        (0..DOMAINS).map(|_| unsafe { Box::from_raw(CapabilityTable::new_boxed()) }).collect();
    // BOOT wires a record into slot 1 of every table and seals it; nothing after
    // the seal may touch or reuse it.
    for t in &tables { t.insert_at(1, D::Domain(7)); t.seal(); }
    let mut m = Model { tables: vec![BTreeMap::new(); DOMAINS as usize], ..Default::default() };
    for t in &mut m.tables { t.insert(1, D::Domain(7)); }

    for op in ops {
        match op {
            Op::Create { owner, notification } => {
                let cap = objects.create(pd(owner));
                // Objects are taken lowest first.
                let expect = (0..).find(|o| !m.objects.contains_key(o)).unwrap();
                let cap = cap.expect("4,096 objects never run out in these runs");
                prop_assert_eq!(cap, ObjectCap { object: expect, generation: m.generation_of(expect), rights: rights::ALL, badge: 0 });
                m.objects.insert(expect, (pd(owner), cap.generation));
                let data = if notification { D::Notification(cap) } else { D::Endpoint(cap) };
                let slot = tables[owner as usize].install(data);
                prop_assert_eq!(slot, Ok(m.first_free(owner as usize)));
                m.tables[owner as usize].insert(slot.unwrap(), data);
            }
            Op::Derive { from, pick, rights: wanted, badge, to } => {
                let held: Vec<(u32, D)> = m.tables[from as usize].iter().map(|(k, v)| (*k, *v)).collect();
                let (slot, data) = held[pick % held.len()];
                let got = tables[from as usize].derive(slot, wanted, badge, objects);
                let expect = match data {
                    D::Domain(_) => Err(MintError::NotHeld),
                    D::Reply(_) => Err(MintError::Rights),
                    _ => {
                        let src = cap_of(&data);
                        if !m.live(&src) { Err(MintError::Stale) }
                        else if src.rights & rights::GRANT == 0 || wanted & !src.rights != 0 { Err(MintError::Rights) }
                        else if src.badge != 0 && badge != src.badge { Err(MintError::Badge) }
                        else {
                            let child = ObjectCap { rights: wanted, badge, ..src };
                            Ok(match data { D::Endpoint(_) => D::Endpoint(child), _ => D::Notification(child) })
                        }
                    }
                };
                prop_assert_eq!(got, expect);
                if let Ok(child) = got {
                    // Authority only narrows, and a badge, once set, is kept.
                    let (src, new) = (cap_of(&data), cap_of(&child));
                    prop_assert!(new.rights & !src.rights == 0);
                    prop_assert!(src.badge == 0 || new.badge == src.badge);
                    let id = tables[to as usize].install(child);
                    prop_assert_eq!(id, Ok(m.first_free(to as usize)));
                    m.tables[to as usize].insert(id.unwrap(), child);
                }
            }
            Op::InstallReply { holder, caller } => {
                // A call hands the server a reply capability carrying the caller's
                // incarnation; the table never lets it derive.
                let reply = D::Reply(ObjectCap { object: pd(caller), generation: objects.incarnation(pd(caller)), rights: rights::ALL, badge: 0 });
                prop_assert_eq!(objects.incarnation(pd(caller)), m.incarnation_of(pd(caller)));
                let id = tables[holder as usize].install(reply);
                prop_assert_eq!(id, Ok(m.first_free(holder as usize)));
                m.tables[holder as usize].insert(id.unwrap(), reply);
            }
            Op::RevokeSlot { domain, pick } => {
                let held: Vec<u32> = m.tables[domain as usize].keys().copied().filter(|&k| k != 1).collect();
                if held.is_empty() { continue; }
                let slot = held[pick % held.len()];
                let gone = tables[domain as usize].revoke(slot).map(|c| c.data);
                prop_assert_eq!(gone, m.tables[domain as usize].remove(&slot));
                // A capability is handed back once.
                prop_assert!(tables[domain as usize].revoke(slot).is_none());
            }
            Op::RevokeObject { pick } => {
                let live: Vec<u32> = m.objects.keys().copied().collect();
                if live.is_empty() {
                    prop_assert!(objects.revoke(pick as u32 % 64).is_none());
                    continue;
                }
                let object = live[pick % live.len()];
                let fresh = objects.revoke(object).expect("live object revokes");
                let g = next_generation(m.objects[&object].1);
                prop_assert_eq!(fresh, ObjectCap { object, generation: g, rights: rights::ALL, badge: 0 });
                m.objects.get_mut(&object).unwrap().1 = g;
                m.generation.insert(object, g);
            }
            Op::Destroy { pick } => {
                let live: Vec<u32> = m.objects.keys().copied().collect();
                if live.is_empty() { prop_assert!(!objects.destroy(pick as u32 % 64)); continue; }
                let object = live[pick % live.len()];
                prop_assert!(objects.destroy(object));
                prop_assert!(!objects.destroy(object));
                m.destroy(object);
            }
            Op::EndIncarnation { domain } => {
                let owned: Vec<u32> = m.objects.iter().filter(|(_, &(o, _))| o == pd(domain)).map(|(k, _)| *k).collect();
                prop_assert_eq!(objects.end_incarnation(pd(domain)), owned.len() as u32);
                for object in owned { m.destroy(object); }
                let g = next_generation(m.incarnation_of(pd(domain)));
                m.incarnation.insert(pd(domain), g);
            }
        }
        check(&m, &tables, objects)?;
    }
    Ok(())
}

fn check(m: &Model, tables: &[Box<CapabilityTable>], objects: &Generations) -> Result<(), TestCaseError> {
    for (domain, (t, held)) in tables.iter().zip(&m.tables).enumerate() {
        let top = held.keys().next_back().copied().unwrap_or(0) + 2;
        for id in 0..=top {
            let got = t.find(id).map(|c| c.data);
            prop_assert_eq!(got, held.get(&id).copied(), "domain {} slot {}", domain, id);
        }
        for data in held.values() {
            match data {
                D::Domain(_) => {}
                D::Reply(cap) => {
                    // A reply is honoured exactly while its caller's incarnation lives.
                    let want = m.incarnation_of(cap.object) == cap.generation;
                    prop_assert_eq!(objects.caller_live(cap.object, cap.generation), want);
                }
                D::Endpoint(cap) | D::Notification(cap) => {
                    prop_assert_eq!(objects.live(cap), m.live(cap), "{:?}", cap);
                }
            }
        }
    }
    for (&object, &(owner, generation)) in &m.objects {
        prop_assert_eq!(objects.owner(object), Some(owner));
        prop_assert!(generation != 0, "generation 0 is never live");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(256), ..ProptestConfig::default() })]
    #[test]
    fn capability_table_matches_reference(ops in prop::collection::vec(op(), 1..80)) {
        run(ops)?;
    }
}

/// A capability copied before its object was revoked never works again, however
/// often the object is revoked, short of the 2^32-step generation wrap.
#[test]
fn revoked_copies_stay_dead_across_many_revocations() {
    let objects = Box::leak(Box::new(Generations::new()));
    let root = objects.create(1).unwrap();
    let copy = root.derive(rights::CALL, 9).unwrap();
    let mut fresh = root;
    for _ in 0..10_000 {
        fresh = objects.revoke(root.object).unwrap();
        assert!(!objects.live(&copy) && !objects.live(&root));
    }
    assert!(objects.live(&fresh));
}

/// The one way a stale capability comes back: the generation is a u32 that skips 0,
/// so 2^32 - 1 revocations of one object return it to an old value. Pinned so a
/// change to the counter is noticed; reaching it takes 4 billion revocations of a
/// single object by its owner (or 4 billion restarts of one domain for a reply).
#[test]
fn generation_wrap_skips_zero_and_is_the_only_revival() {
    assert_eq!(next_generation(u32::MAX), 1);
    assert_eq!(next_generation(1), 2);
    assert_eq!(next_generation(u32::MAX - 1), u32::MAX);
}
