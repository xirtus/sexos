// SPDX-License-Identifier: GPL-3.0-only
//! RAM-only expiring state for link flows (leeloo-multipass kept these sealed in Postgres).
//! A flow is single-use: [`Expiring::consume`] removes it, so a replayed callback fails.
use alloc::collections::BTreeMap;
use alloc::string::String;

pub struct Expiring<T> { items: BTreeMap<String, (u64, T)>, ttl: u64, limit: usize }

impl<T> Expiring<T> {
    /// `ttl` in seconds (leeloo-multipass: 1200 for flows, 3600 for profile proofs).
    pub fn new(ttl: u64, limit: usize) -> Self { Self { items: BTreeMap::new(), ttl, limit } }

    pub fn set(&mut self, id: &str, value: T, now: u64) -> bool {
        self.purge(now);
        if self.items.len() >= self.limit && !self.items.contains_key(id) { return false; }
        self.items.insert(id.into(), (now + self.ttl, value));
        true
    }
    pub fn get(&self, id: &str, now: u64) -> Option<&T> {
        self.items.get(id).filter(|(exp, _)| *exp > now).map(|(_, v)| v)
    }
    pub fn consume(&mut self, id: &str, now: u64) -> Option<T> {
        let (exp, v) = self.items.remove(id)?;
        (exp > now).then_some(v)
    }
    pub fn purge(&mut self, now: u64) { self.items.retain(|_, (exp, _)| *exp > now); }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::Expiring;
    #[test]
    fn expires_and_is_single_use() {
        let mut s = Expiring::new(1200, 2);
        assert!(s.set("a", 1, 100));
        assert_eq!(s.get("a", 1299), Some(&1));
        assert_eq!(s.get("a", 1300), None);
        assert_eq!(s.consume("a", 500), Some(1));
        assert_eq!(s.consume("a", 500), None);
        s.set("b", 2, 0); s.set("c", 3, 0);
        assert!(!s.set("d", 4, 10));
        assert!(s.set("d", 4, 1200)); // the others expired and were purged
        assert_eq!(s.len(), 1);
    }
}
