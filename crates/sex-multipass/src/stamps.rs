// SPDX-License-Identifier: GPL-3.0-only
//! The stamp book: every network identity a person has added. Solid stamps are verified
//! (a provider proved ownership and gave a stable subject id); dashed stamps are saved
//! addresses. Ported from leeloo-multipass `saveNetwork()` and `001_core.sql`.
//!
//! The book holds no secrets, so it is kept as a plain file in Files. Tokens are in
//! Shibari and are referenced only from RAM.
use crate::address::{NetworkAccount, Provider};
use crate::{ApiError, Result};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub id: u64,
    pub principal: String,
    pub account: NetworkAccount,
    /// Provider's stable id (account id, DID, MXID, actor URL). Only verified stamps have one.
    pub subject: Option<String>,
    pub added_at: u64,
    pub verified_at: Option<u64>,
}

impl Stamp { pub fn verified(&self) -> bool { self.verified_at.is_some() } }

#[derive(Default)]
pub struct StampBook { stamps: Vec<Stamp>, next: u64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event { Saved, Verified, Removed }

impl StampBook {
    pub fn new() -> Self { Self { stamps: Vec::new(), next: 1 } }

    pub fn list<'a>(&'a self, principal: &'a str) -> impl Iterator<Item = &'a Stamp> + 'a {
        self.stamps.iter().filter(move |s| s.principal == principal)
    }
    pub fn get(&self, id: u64) -> Option<&Stamp> { self.stamps.iter().find(|s| s.id == id) }

    /// Save an address, or verify it when `subject` is given.
    pub fn save(&mut self, principal: &str, account: NetworkAccount, subject: Option<&str>, now: u64) -> Result<(u64, Event)> {
        if principal.trim().is_empty() || principal.len() > 200 {
            return Err(ApiError::new("unauthenticated", "Sign in before changing your stamps."));
        }
        if let Some(subject) = subject {
            if let Some(i) = self.stamps.iter().position(|s| s.verified() && s.account.provider == account.provider
                && s.account.network == account.network && s.subject.as_deref() == Some(subject)) {
                if self.stamps[i].principal != principal {
                    return Err(ApiError::new("network_already_linked", "That verified account belongs to another Passport. Disconnect it there first."));
                }
                let id = self.stamps[i].id;
                self.stamps.retain(|s| !(s.principal == principal && s.account.provider == account.provider
                    && s.account.address == account.address && s.id != id && !s.verified()));
                let s = self.stamps.iter_mut().find(|s| s.id == id).unwrap();
                s.account.address = account.address;
                s.account.profile_url = account.profile_url;
                s.verified_at = Some(now);
                return Ok((id, Event::Verified));
            }
        }
        if let Some(s) = self.stamps.iter_mut().find(|s| s.principal == principal
            && s.account.provider == account.provider && s.account.address == account.address) {
            s.account.profile_url = account.profile_url;
            if let Some(sub) = subject { s.subject = Some(sub.into()); s.verified_at = Some(now); return Ok((s.id, Event::Verified)); }
            return Ok((s.id, Event::Saved));
        }
        let id = self.next; self.next += 1;
        let verified = subject.is_some();
        self.stamps.push(Stamp { id, principal: principal.into(), account, subject: subject.map(Into::into), added_at: now, verified_at: verified.then_some(now) });
        Ok((id, if verified { Event::Verified } else { Event::Saved }))
    }

    pub fn remove(&mut self, principal: &str, id: u64) -> bool {
        let before = self.stamps.len();
        self.stamps.retain(|s| !(s.id == id && s.principal == principal));
        before != self.stamps.len()
    }

    /// One stamp per line, tab-separated. Fields never contain tabs or newlines (addresses
    /// and URLs are validated before they get here); any that do make the load fail.
    pub fn to_text(&self) -> String {
        let mut out = String::from("multipass-stamps 1\n");
        for s in &self.stamps {
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", s.id, s.principal, s.account.provider.as_str(),
                s.account.network, s.account.address, s.account.profile_url, s.subject.as_deref().unwrap_or("-"),
                s.added_at, s.verified_at.map(|v| v.to_string()).unwrap_or_else(|| "-".into())));
        }
        out
    }

    pub fn from_text(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "multipass-stamps 1" { return None; }
        let mut book = Self::new();
        for line in lines.filter(|l| !l.is_empty()) {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 9 { return None; }
            let id: u64 = f[0].parse().ok()?;
            let opt = |v: &str| (v != "-").then(|| v.to_string());
            book.stamps.push(Stamp {
                id, principal: f[1].into(),
                account: NetworkAccount { provider: Provider::parse(f[2])?, network: f[3].into(), address: f[4].into(), profile_url: f[5].into() },
                subject: opt(f[6]), added_at: f[7].parse().ok()?,
                verified_at: match f[8] { "-" => None, v => Some(v.parse().ok()?) },
            });
            book.next = book.next.max(id + 1);
        }
        Some(book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::network_account;

    fn masto(a: &str) -> NetworkAccount { network_account(Provider::Mastodon, a).unwrap() }

    #[test]
    fn saved_then_verified_and_owned_once() {
        let mut b = StampBook::new();
        let (id, e) = b.save("me", masto("@alice@social.example"), None, 10).unwrap();
        assert_eq!(e, Event::Saved);
        assert!(!b.get(id).unwrap().verified());
        let (id2, e) = b.save("me", masto("@alice@social.example"), Some("109"), 20).unwrap();
        assert_eq!((id2, e), (id, Event::Verified));
        assert_eq!(b.get(id).unwrap().verified_at, Some(20));
        // Another passport cannot claim the same verified subject.
        assert_eq!(b.save("you", masto("@alice@social.example"), Some("109"), 30).unwrap_err().code, "network_already_linked");
        // The account was renamed: the verified stamp follows the subject, the dashed duplicate goes.
        b.save("me", masto("@alice2@social.example"), None, 40).unwrap();
        let (id3, _) = b.save("me", masto("@alice2@social.example"), Some("109"), 50).unwrap();
        assert_eq!(id3, id);
        assert_eq!(b.list("me").count(), 1);
        assert_eq!(b.get(id).unwrap().account.address, "@alice2@social.example");
    }

    #[test]
    fn text_round_trip() {
        let mut b = StampBook::new();
        b.save("me", masto("@a@s.example"), Some("1"), 5).unwrap();
        b.save("me", network_account(Provider::Bluesky, "a.bsky.social").unwrap(), None, 6).unwrap();
        let t = b.to_text();
        let c = StampBook::from_text(&t).unwrap();
        assert_eq!(c.to_text(), t);
        assert_eq!(c.get(1).unwrap().verified_at, Some(5));
        assert!(StampBook::from_text("garbage").is_none());
        assert!(b.remove("me", 2) && !b.remove("you", 1));
    }
}
