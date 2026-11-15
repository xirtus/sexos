// SPDX-License-Identifier: GPL-3.0-only
//! Recent files ranked by frecency, the way Firefox ranks its address bar.
//!
//! Each file keeps how often it was visited and its last ten visits. A visit counts by
//! its age (within 4 days 100, 14 days 70, 31 days 50, 90 days 30, older 10) and its kind
//! (opening 100%, saving 150%, pinning 200%); the score is the visit count times the
//! average of those sampled visits. Old favourites fade, a file used often today rises.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::date::DAY;
use crate::text::{escape, lines, parse_u64, push_u64, split_unescaped, unescape};
use crate::{is_within, rebase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visit { Open, Save, Pin }

impl Visit {
    fn bonus(self) -> u64 { match self { Visit::Open => 100, Visit::Save => 150, Visit::Pin => 200 } }
    fn code(self) -> u8 { match self { Visit::Open => b'o', Visit::Save => b's', Visit::Pin => b'p' } }
    fn from_code(c: u8) -> Option<Visit> { match c { b'o' => Some(Visit::Open), b's' => Some(Visit::Save), b'p' => Some(Visit::Pin), _ => None } }
}

/// How much a visit `age` seconds old counts.
pub fn bucket(age: u64) -> u64 {
    match age / DAY { 0..=3 => 100, 4..=13 => 70, 14..=30 => 50, 31..=89 => 30, _ => 10 }
}

const SAMPLES: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Entry { count: u32, visits: Vec<(u64, Visit)> }

impl Entry {
    fn score(&self, now: u64) -> u64 {
        if self.visits.is_empty() { return 0; }
        let sum: u64 = self.visits.iter().map(|(t, k)| bucket(now.saturating_sub(*t)) * k.bonus() / 100).sum();
        (self.count as u64 * sum).div_ceil(self.visits.len() as u64)
    }
    fn last(&self) -> u64 { self.visits.iter().map(|v| v.0).max().unwrap_or(0) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent { map: BTreeMap<Vec<u8>, Entry>, /// Most files remembered.
    pub max: usize }

impl Default for Recent { fn default() -> Self { Recent { map: BTreeMap::new(), max: 500 } } }

const HEADER: &[u8] = b"loom-recent 1";

impl Recent {
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }

    pub fn visit(&mut self, path: &[u8], now: u64, kind: Visit) {
        let e = self.map.entry(path.to_vec()).or_default();
        e.count = e.count.saturating_add(1);
        e.visits.push((now, kind));
        if e.visits.len() > SAMPLES { e.visits.remove(0); }
        if self.map.len() > self.max {
            // Forget the weakest; never the one just visited.
            let weakest = self.map.iter().filter(|(p, _)| &p[..] != path).min_by_key(|(p, e)| (e.score(now), e.last(), *p)).map(|(p, _)| p.clone());
            if let Some(w) = weakest { self.map.remove(&w); }
        }
    }

    pub fn score(&self, path: &[u8], now: u64) -> u64 { self.map.get(path).map_or(0, |e| e.score(now)) }

    /// The `n` best files, best first (ties: more recent first, then by path).
    pub fn top(&self, now: u64, n: usize) -> Vec<(Vec<u8>, u64)> {
        let mut v: Vec<(Vec<u8>, u64, u64)> = self.map.iter().map(|(p, e)| (p.clone(), e.score(now), e.last())).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then_with(|| a.0.cmp(&b.0)));
        v.into_iter().take(n).map(|(p, s, _)| (p, s)).collect()
    }

    pub fn moved(&mut self, from: &[u8], to: &[u8]) {
        let keys: Vec<Vec<u8>> = self.map.keys().filter(|k| is_within(k, from)).cloned().collect();
        for k in keys { if let Some(e) = self.map.remove(&k) { self.map.insert(rebase(&k, from, to), e); } }
    }
    pub fn removed(&mut self, path: &[u8]) { self.map.retain(|k, _| !is_within(k, path)); }

    /// `path<TAB>count<TAB>time:kind,time:kind...` per file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(b'\n');
        for (p, e) in &self.map {
            escape(p, &mut out);
            out.push(b'\t');
            push_u64(&mut out, e.count as u64);
            out.push(b'\t');
            for (i, (t, k)) in e.visits.iter().enumerate() {
                if i > 0 { out.push(b','); }
                push_u64(&mut out, *t);
                out.push(b':');
                out.push(k.code());
            }
            out.push(b'\n');
        }
        out
    }

    pub fn from_bytes(raw: &[u8]) -> Option<Recent> {
        let mut it = lines(raw);
        if it.next()? != HEADER { return None; }
        let mut r = Recent::default();
        for l in it {
            let f = split_unescaped(l, b'\t');
            if f.len() != 3 { continue; }
            let Some(count) = parse_u64(f[1]) else { continue };
            let visits: Option<Vec<(u64, Visit)>> = f[2].split(|b| *b == b',').map(|v| {
                let (t, k) = v.split_at(v.iter().position(|b| *b == b':')?);
                Some((parse_u64(t)?, Visit::from_code(*k.get(1)?)?))
            }).collect();
            let Some(mut visits) = visits else { continue };
            visits.truncate(SAMPLES);
            r.map.insert(unescape(f[0]), Entry { count: count.min(u32::MAX as u64) as u32, visits });
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: u64 = 1_790_000_000;

    #[test]
    fn frecency_ranks() {
        let mut r = Recent::default();
        // Opened 20 times two months ago.
        for i in 0..20 { r.visit(b"old-favourite", NOW - 60 * DAY + i, Visit::Open); }
        // Opened 3 times today.
        for i in 0..3 { r.visit(b"today", NOW - i, Visit::Open); }
        // Once a week ago, saved.
        r.visit(b"saved", NOW - 7 * DAY, Visit::Save);
        // Golden scores: count x average weight.
        assert_eq!(r.score(b"old-favourite", NOW), 20 * 30);
        assert_eq!(r.score(b"today", NOW), 300);
        assert_eq!(r.score(b"saved", NOW), 105);
        assert_eq!(r.score(b"nothing", NOW), 0);
        let top = r.top(NOW, 2);
        assert_eq!(top[0].0, b"old-favourite");
        assert_eq!(top[1].0, b"today");
        // A month on both fade: today's visits weigh 30 each, the favourite's 10.
        let later = NOW + 31 * DAY;
        assert_eq!(r.score(b"today", later), 90);
        assert_eq!(r.score(b"old-favourite", later), 200);
        // Using "today" more makes it overtake.
        for i in 0..5 { r.visit(b"today", NOW + i, Visit::Open); }
        assert_eq!(r.top(NOW, 1)[0].0, b"today");
        assert_eq!(bucket(0), 100);
        assert_eq!(bucket(4 * DAY), 70);
        assert_eq!(bucket(365 * DAY), 10);
    }

    #[test]
    fn bounded_moves_and_round_trip() {
        let mut r = Recent { max: 3, ..Recent::default() };
        r.visit(b"a/1", NOW - 100 * DAY, Visit::Open);
        r.visit(b"a/2", NOW, Visit::Pin);
        r.visit(b"b", NOW, Visit::Open);
        r.visit(b"c", NOW, Visit::Save);
        assert_eq!(r.len(), 3);
        assert_eq!(r.score(b"a/1", NOW), 0, "the weakest was forgotten");
        r.moved(b"a", b"z");
        assert_eq!(r.score(b"z/2", NOW), 200);
        let back = Recent::from_bytes(&r.to_bytes()).unwrap();
        assert_eq!(back.map, r.map);
        r.removed(b"z");
        assert_eq!(r.len(), 2);
        for i in 0..15 { r.visit(b"b", NOW + i, Visit::Open); }
        assert_eq!(r.map[&b"b".to_vec()].visits.len(), SAMPLES);
        assert_eq!(r.map[&b"b".to_vec()].count, 16);
    }
}
