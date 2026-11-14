// SPDX-License-Identifier: GPL-3.0-only
//! Conversation threading: messages join through Message-ID, In-Reply-To and References
//! (the JWZ idea, without its container tree), and replies whose headers were stripped
//! fall back to the same normalised subject.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

pub struct Item<'a> { pub id: Option<&'a str>, pub parents: Vec<&'a str>, pub subject: &'a str }

/// `Re: Fwd: AW: [list] Hello` → `hello`.
pub fn base_subject(s: &str) -> String {
    let mut t = s.trim();
    loop {
        let before = t;
        if t.starts_with('[') { if let Some(e) = t.find(']') { t = t[e + 1..].trim_start(); } }
        for p in ["re:", "fw:", "fwd:", "aw:", "sv:", "antw:"] {
            if t.len() >= p.len() && t[..p.len()].eq_ignore_ascii_case(p) { t = t[p.len()..].trim_start(); }
        }
        if t == before { break; }
    }
    t.to_lowercase()
}

/// Returns a thread number for each item; items with the same number are one conversation.
pub fn thread(items: &[Item]) -> Vec<usize> {
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize { while p[i] != i { p[i] = p[p[i]]; i = p[i]; } i }
    let mut by_id: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() { if let Some(id) = it.id { by_id.entry(id).or_insert(i); } }
    // Message IDs that are referenced but absent still join their children.
    let mut phantom: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        for p in &it.parents {
            let j = match by_id.get(p) { Some(&j) => j, None => *phantom.entry(p).or_insert(i) };
            let (a, b) = (root(&mut parent, i), root(&mut parent, j));
            if a != b { parent[a] = b; }
        }
    }
    let mut by_subject: BTreeMap<String, usize> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        if !it.parents.is_empty() { continue; }
        let s = base_subject(it.subject);
        if s.is_empty() { continue; }
        let replyish = base_subject(it.subject) != it.subject.trim().to_lowercase();
        match by_subject.get(&s) {
            Some(&j) if replyish => { let (a, b) = (root(&mut parent, i), root(&mut parent, j)); if a != b { parent[a] = b; } }
            Some(_) => {}
            None => { by_subject.insert(s, i); }
        }
    }
    let mut number: BTreeMap<usize, usize> = BTreeMap::new();
    (0..n).map(|i| { let r = root(&mut parent, i); let next = number.len(); *number.entry(r).or_insert(next) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn joins_by_headers_and_reply_subjects() {
        let items = [
            Item { id: Some("a"), parents: vec![], subject: "Dinner" },
            Item { id: Some("b"), parents: vec!["a"], subject: "Re: Dinner" },
            Item { id: Some("c"), parents: vec!["missing", "b"], subject: "Re: Dinner" },
            Item { id: Some("d"), parents: vec![], subject: "Taxes" },
            Item { id: Some("e"), parents: vec![], subject: "AW: [family] Re: dinner" },
            Item { id: Some("f"), parents: vec!["missing"], subject: "unrelated words" },
            Item { id: Some("g"), parents: vec![], subject: "Taxes" },
        ];
        let t = thread(&items);
        assert_eq!(t[0], t[1]); assert_eq!(t[1], t[2]); assert_eq!(t[2], t[4]); assert_eq!(t[2], t[5]);
        assert_ne!(t[0], t[3]);
        assert_ne!(t[3], t[6], "two fresh messages with one subject stay apart");
        assert_eq!(base_subject("Re: RE: Fwd: Hello"), "hello");
    }
}
