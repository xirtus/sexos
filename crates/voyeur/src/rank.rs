// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Matching and frecency ranking. Integer only (no floats in the Shell).
//!
//! A result's score is
//!
//! ```text
//! base   = max(match(query, title, keywords), min(provider hint, hint cap))
//! score  = base * provider weight / 100 + frecency(kind, title)
//! ```
//!
//! The Shell computes `match` itself, so a provider cannot lift an unrelated
//! result by naming it well; a provider's own hint (Abacus answering `2+2`
//! with `4`, which does not contain the query) counts only up to the cap
//! the Shell gives that provider. Results with base 0 are dropped.

/// The query equals the title (ignoring ASCII case).
pub const EXACT: u32 = 1000;
/// The title starts with the query.
pub const PREFIX: u32 = 800;
/// Each word of the query starts a word of the title, in order.
pub const WORD_PREFIX: u32 = 600;
/// The query appears inside the title.
pub const SUBSTRING: u32 = 400;
/// The query equals one of the result's search words.
pub const KEYWORD_EXACT: u32 = 350;
/// The query starts one of the result's search words.
pub const KEYWORD_PREFIX: u32 = 300;
/// Every query character appears in order, the first at a word start.
pub const FUZZY: u32 = 100;
/// Most frecency adds: a well-used prefix match can pass an unused exact one.
pub const FRECENCY_MAX: u32 = 300;
/// The highest hint an external program's result counts with: below a
/// Shell-checked prefix match of the query, so it can never bury one.
pub const HINT_CAP_EXTERNAL: u16 = 600;
/// The highest hint a Shell built-in provider counts with (Abacus).
pub const HINT_CAP_BUILTIN: u16 = 2000;

fn trim(q: &str) -> &[u8] {
    let b = q.as_bytes();
    let start = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    let end = b.iter().rposition(|&c| c != b' ').map_or(start, |e| e + 1);
    &b[start..end]
}
fn eq_ci(a: &[u8], b: &[u8]) -> bool { a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y)) }
fn starts_ci(t: &[u8], p: &[u8]) -> bool { t.len() >= p.len() && eq_ci(&t[..p.len()], p) }
fn find_ci(t: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() || n.len() > t.len() { return None; }
    (0..=t.len() - n.len()).find(|&i| eq_ci(&t[i..i + n.len()], n))
}
fn word_char(b: u8) -> bool { b.is_ascii_alphanumeric() || b >= 0x80 }
fn word_start(t: &[u8], i: usize) -> bool { word_char(t[i]) && (i == 0 || !word_char(t[i - 1])) }
fn closeness(q: usize, t: usize) -> u32 { (q * 99 / t.max(1)).min(99) as u32 }

/// Each space-separated query word starts a word of the title, in order.
fn word_prefix(q: &[u8], t: &[u8]) -> bool {
    let mut at = 0usize;
    let mut any = false;
    for w in q.split(|&c| c == b' ').filter(|w| !w.is_empty()) {
        let Some(i) = (at..t.len()).find(|&i| word_start(t, i) && starts_ci(&t[i..], w)) else { return false };
        at = i + w.len();
        any = true;
    }
    any
}

/// Subsequence match; 0..=99 quality or None. The first character must
/// start a word, which keeps one-letter noise out.
fn fuzzy(q: &[u8], t: &[u8]) -> Option<u32> {
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    let mut quality = 0u32;
    let mut n = 0u32;
    for &c in q.iter().filter(|&&c| c != b' ') {
        let i = if last.is_none() {
            (0..t.len()).find(|&i| word_start(t, i) && t[i].eq_ignore_ascii_case(&c))?
        } else {
            (at..t.len()).find(|&i| t[i].eq_ignore_ascii_case(&c))?
        };
        if word_start(t, i) { quality += 12; }
        if last.is_some_and(|l| l + 1 == i) { quality += 6; }
        last = Some(i);
        at = i + 1;
        n += 1;
    }
    if n < 2 { return None; }
    Some(quality.min(99))
}

/// How well `query` names `title`, ignoring ASCII case; 0 for no match.
pub fn title_score(query: &str, title: &str) -> u32 {
    let (q, t) = (trim(query), title.as_bytes());
    if q.is_empty() || t.is_empty() { return 0; }
    if eq_ci(q, t) { return EXACT; }
    let c = closeness(q.len(), t.len());
    if starts_ci(t, q) { return PREFIX + c; }
    if word_prefix(q, t) { return WORD_PREFIX + c; }
    if find_ci(t, q).is_some() { return SUBSTRING + c; }
    fuzzy(q, t).map_or(0, |f| FUZZY + f)
}

/// How well `query` names one of the space-separated `keywords`.
pub fn keyword_score(query: &str, keywords: &str) -> u32 {
    let q = trim(query);
    if q.is_empty() { return 0; }
    keywords.as_bytes().split(|&c| c == b' ').filter(|w| !w.is_empty()).map(|w| {
        if eq_ci(w, q) { KEYWORD_EXACT } else if starts_ci(w, q) { KEYWORD_PREFIX } else { 0 }
    }).max().unwrap_or(0)
}

/// The Shell's own judgement of how well a result matches the query.
pub fn match_score(query: &str, title: &str, keywords: &str) -> u32 {
    title_score(query, title).max(keyword_score(query, keywords))
}

/// Frecency identity of a result: its kind and title, so choosing "Open
/// Quil" counts the same whichever provider offered it.
pub fn key(kind: u8, title: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ kind as u64;
    h = h.wrapping_mul(0x100_0000_01b3);
    for &b in title.as_bytes() { h = (h ^ b.to_ascii_lowercase() as u64).wrapping_mul(0x100_0000_01b3); }
    h
}

/// How much one use still counts after `age` seconds, out of 100.
pub fn decay(age: u64) -> u32 {
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    match age {
        a if a < 4 * HOUR => 100,
        a if a < DAY => 80,
        a if a < 7 * DAY => 60,
        a if a < 30 * DAY => 40,
        a if a < 90 * DAY => 20,
        _ => 10,
    }
}

/// Uses one table remembers. When full, the use worth least now goes.
pub const FRECENCY_SLOTS: usize = 32;
/// Uses counted per result; more than this ranks no higher.
pub const COUNT_CAP: u16 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Use { key: u64, count: u16, last: u64 }

/// Launch counts with time decay (Firefox's frecency, bucketed by age).
/// Times are seconds on any clock that only goes forward.
#[derive(Clone, Copy, Debug)]
pub struct Frecency { uses: [Use; FRECENCY_SLOTS] }

impl Frecency {
    pub const fn new() -> Self { Self { uses: [Use { key: 0, count: 0, last: 0 }; FRECENCY_SLOTS] } }
    /// The user chose the result named by `key` at `now`.
    pub fn record(&mut self, key: u64, now: u64) {
        if let Some(u) = self.uses.iter_mut().find(|u| u.count != 0 && u.key == key) {
            u.count = u.count.saturating_add(1);
            u.last = u.last.max(now);
            return;
        }
        let slot = match self.uses.iter().position(|u| u.count == 0) {
            Some(i) => i,
            None => (0..FRECENCY_SLOTS).min_by_key(|&i| (self.value(&self.uses[i], now), self.uses[i].last)).unwrap_or(0),
        };
        self.uses[slot] = Use { key, count: 1, last: now };
    }
    fn value(&self, u: &Use, now: u64) -> u32 {
        if u.count == 0 { return 0; }
        (u.count.min(COUNT_CAP) as u32 * decay(now.saturating_sub(u.last)) / 10).min(FRECENCY_MAX)
    }
    /// What frecency adds to the result named by `key` at `now`.
    pub fn score(&self, key: u64, now: u64) -> u32 {
        self.uses.iter().find(|u| u.count != 0 && u.key == key).map_or(0, |u| self.value(u, now))
    }
    pub fn count(&self, key: u64) -> u16 {
        self.uses.iter().find(|u| u.count != 0 && u.key == key).map_or(0, |u| u.count)
    }
}

impl Default for Frecency { fn default() -> Self { Self::new() } }

/// One provider's result as the Shell sees it.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    /// The provider's place in the Shell's provider list; earlier wins ties.
    pub provider: u8,
    /// Percent the provider's matches count with (100 normal, 0..=200).
    pub weight: u8,
    pub kind: u8,
    pub title: &'a str,
    /// Space-separated words the result is also found by.
    pub keywords: &'a str,
    /// The provider's own score for a result the query text does not name.
    pub hint: u16,
    /// The most `hint` may count with ([`HINT_CAP_EXTERNAL`] for programs).
    pub hint_cap: u16,
}

/// A merged result: the candidate's index and its final score.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ranked { pub index: usize, pub score: u32 }

/// A candidate's final score; 0 drops it.
pub fn score(query: &str, now: u64, frecency: &Frecency, c: &Candidate<'_>) -> u32 {
    let base = match_score(query, c.title, c.keywords).max(c.hint.min(c.hint_cap) as u32);
    if base == 0 { return 0; }
    base * c.weight.min(200) as u32 / 100 + frecency.score(key(c.kind, c.title), now)
}

/// Most candidates one merge looks at.
pub const MERGE_MAX: usize = 128;

/// Merges every provider's candidates into `out`, best first, and returns
/// how many there are. Order: score, then provider order, then the shorter
/// title, then the order candidates came in. A result two providers both
/// offer (same kind and title) appears once, at its better score. An
/// empty query has no results.
pub fn merge(query: &str, now: u64, frecency: &Frecency, candidates: &[Candidate<'_>], out: &mut [Ranked]) -> usize {
    if trim(query).is_empty() { return 0; }
    let mut all = [Ranked::default(); MERGE_MAX];
    let mut n = 0usize;
    for (index, c) in candidates.iter().enumerate().take(MERGE_MAX) {
        let s = score(query, now, frecency, c);
        if s == 0 { continue; }
        let k = key(c.kind, c.title);
        if let Some(d) = all[..n].iter_mut().find(|r| key(candidates[r.index].kind, candidates[r.index].title) == k) {
            if s > d.score { *d = Ranked { index, score: s }; }
            continue;
        }
        all[n] = Ranked { index, score: s };
        n += 1;
    }
    let better = |a: &Ranked, b: &Ranked| -> bool {
        let (ca, cb) = (&candidates[a.index], &candidates[b.index]);
        (a.score, core::cmp::Reverse(ca.provider), core::cmp::Reverse(ca.title.len()), core::cmp::Reverse(a.index))
            > (b.score, core::cmp::Reverse(cb.provider), core::cmp::Reverse(cb.title.len()), core::cmp::Reverse(b.index))
    };
    // Insertion sort: n is small and this must not allocate.
    for i in 1..n {
        let mut j = i;
        while j > 0 && better(&all[j], &all[j - 1]) { all.swap(j, j - 1); j -= 1; }
    }
    let shown = n.min(out.len());
    out[..shown].copy_from_slice(&all[..shown]);
    shown
}
