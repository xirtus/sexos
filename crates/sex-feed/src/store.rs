// SPDX-License-Identifier: GPL-3.0-only
//! What Gossip remembers, as text files in its folder:
//!
//! * `feeds.txt`: one subscription a line, `URL<TAB>TITLE`.
//! * `read.txt`: one read item a line, `FEED-HASH<TAB>ITEM-ID`, oldest first, at most
//!   [`READ_MAX`] lines (the oldest are forgotten first).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const READ_MAX: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription { pub url: String, pub title: String }

fn clean(s: &str) -> String { s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect() }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subscriptions { pub feeds: Vec<Subscription> }

impl Subscriptions {
    pub fn parse(text: &str) -> Subscriptions {
        let mut s = Subscriptions::default();
        for line in text.lines() {
            let (url, title) = line.split_once('\t').unwrap_or((line, ""));
            s.add(url, title);
        }
        s
    }
    pub fn serialize(&self) -> String {
        self.feeds.iter().map(|f| format!("{}\t{}\n", f.url, f.title)).collect()
    }
    /// Adds a feed unless it is there already; true when added.
    pub fn add(&mut self, url: &str, title: &str) -> bool {
        let url = clean(url.trim());
        if url.is_empty() || self.feeds.iter().any(|f| f.url == url) { return false; }
        self.feeds.push(Subscription { url, title: clean(title.trim()) });
        true
    }
    /// Adds every new one; how many were new.
    pub fn merge(&mut self, subs: &[Subscription]) -> usize { subs.iter().filter(|s| self.add(&s.url, &s.title)).count() }
    pub fn remove(&mut self, url: &str) { self.feeds.retain(|f| f.url != url); }
    /// Sets the title of a feed, as its document gave it; true when it changed.
    pub fn retitle(&mut self, url: &str, title: &str) -> bool {
        let title = clean(title.trim());
        match self.feeds.iter_mut().find(|f| f.url == url) {
            Some(f) if !title.is_empty() && f.title != title => { f.title = title; true }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadState { read: Vec<(String, String)> }

impl ReadState {
    pub fn parse(text: &str) -> ReadState {
        let read = text.lines().filter_map(|l| l.split_once('\t')).map(|(f, i)| (String::from(f), String::from(i))).collect();
        ReadState { read }
    }
    pub fn serialize(&self) -> String { self.read.iter().map(|(f, i)| format!("{f}\t{i}\n")).collect() }
    pub fn len(&self) -> usize { self.read.len() }
    pub fn is_empty(&self) -> bool { self.read.is_empty() }
    pub fn is_read(&self, feed: &str, id: &str) -> bool {
        let (f, i) = (crate::hash(feed), clean(id));
        self.read.iter().any(|(a, b)| *a == f && *b == i)
    }
    /// Marks an item read or unread; true when that changed anything.
    pub fn set(&mut self, feed: &str, id: &str, read: bool) -> bool {
        let (f, i) = (crate::hash(feed), clean(id));
        let at = self.read.iter().position(|(a, b)| *a == f && *b == i);
        match (at, read) {
            (Some(_), true) | (None, false) => false,
            (None, true) => {
                self.read.push((f, i));
                if self.read.len() > READ_MAX { let extra = self.read.len() - READ_MAX; self.read.drain(..extra); }
                true
            }
            (Some(k), false) => { self.read.remove(k); true }
        }
    }
    /// Forgets everything read in a feed (when it is unsubscribed).
    pub fn forget(&mut self, feed: &str) { let f = crate::hash(feed); self.read.retain(|(a, _)| *a != f); }
}
