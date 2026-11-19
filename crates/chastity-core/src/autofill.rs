// SPDX-License-Identifier: GPL-3.0-only
//! The boundary a browser fills logins through. Nothing calls it yet.
//!
//! **Plan for Kaleidoscope** (not wired; no PDX opcode, Shell route or catalog policy
//! exists for it yet):
//!
//! 1. Kaleidoscope asks, over a Shell-brokered route, "logins for this page", giving
//!    only the page's URL. It never names an entry and never gets a list of the vault.
//! 2. Chastity, unlocked, answers with [`candidates`]: titles and user names of entries
//!    whose URL has the page's origin (scheme, host, port). An `http` page never matches
//!    an `https` entry, and a subdomain does not match its parent.
//! 3. The person picks one *in Chastity's window* (or confirms the only one). Only then
//!    Chastity makes a [`Fill`] with [`fill`]: one entry, one origin, used once.
//! 4. The fill crosses to Kaleidoscope like a clipboard publish (bounded, register-only,
//!    erased after the ack) and Kaleidoscope types it into the focused form.
//!
//! So the browser holds no standing authority over the vault: each fill is one grant the
//! person gave, for the page they are on. When Shibari exists, step 3's secret can come
//! straight from it to the browser without passing through Chastity's heap.
use zeroize::Zeroizing;

use crate::vault::{EntryId, Vault};

/// Scheme, host and port of a URL, lowercased; the default port when none is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin { pub scheme: String, pub host: String, pub port: u16 }

impl Origin {
    pub fn parse(url: &str) -> Option<Origin> {
        let (scheme, rest) = url.trim().split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default = match scheme.as_str() { "https" => 443, "http" => 80, _ => return None };
        let authority = rest.split(['/', '?', '#']).next()?;
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || p.is_empty() => (h, p.parse().ok()?),
            _ => (authority, default),
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        (!host.is_empty()).then_some(Origin { scheme, host, port })
    }
}

/// What the browser may see before the person chooses: no secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate { pub id: EntryId, pub title: String, pub username: String }

/// Entries for a page, by exact origin.
pub fn candidates(vault: &Vault, page_url: &str) -> Vec<Candidate> {
    let Some(page) = Origin::parse(page_url) else { return Vec::new() };
    vault.entries().into_iter()
        .filter(|s| Origin::parse(&s.url).as_ref() == Some(&page))
        .map(|s| Candidate { id: s.id, title: s.title, username: s.username })
        .collect()
}

/// One login, for one origin, once: the person chose it in Chastity.
pub struct Fill {
    pub origin: Origin,
    pub username: String,
    pub password: Zeroizing<String>,
}

/// The fill the person chose: refused unless the entry is one of the page's candidates.
pub fn fill(vault: &Vault, page_url: &str, chosen: EntryId) -> Option<Fill> {
    if !candidates(vault, page_url).iter().any(|c| c.id == chosen) { return None; }
    let d = vault.detail(chosen)?;
    Some(Fill { origin: Origin::parse(page_url)?, username: d.summary.username.clone(), password: d.password })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{Kdf, NewEntry};

    #[test]
    fn origins() {
        let o = |u| Origin::parse(u);
        assert_eq!(o("https://Mail.Example.org/login?x=1"), Some(Origin { scheme: "https".into(), host: "mail.example.org".into(), port: 443 }));
        assert_eq!(o("http://user:pw@example.org:8080/"), Some(Origin { scheme: "http".into(), host: "example.org".into(), port: 8080 }));
        assert_eq!(o("example.org"), None);
        assert_eq!(o("ftp://example.org"), None);
    }

    #[test]
    fn only_the_exact_origin_and_only_a_chosen_candidate() {
        let mut v = Vault::create("v", &mut "p".to_string(), Kdf::TEST_ONLY);
        let mail = v.add(NewEntry { title: "Mail".into(), username: "me".into(), password: Zeroizing::new("pw".into()), url: "https://mail.example.org/".into(), ..Default::default() });
        let other = v.add(NewEntry { title: "Other".into(), url: "https://example.org".into(), ..Default::default() });
        let page = "https://mail.example.org/inbox";
        assert_eq!(candidates(&v, page), vec![Candidate { id: mail, title: "Mail".into(), username: "me".into() }]);
        assert!(candidates(&v, "http://mail.example.org/").is_empty(), "no https login on an http page");
        assert!(candidates(&v, "https://evil.mail.example.org/").is_empty());
        let f = fill(&v, page, mail).unwrap();
        assert_eq!((f.username.as_str(), f.password.as_str(), f.origin.host.as_str()), ("me", "pw", "mail.example.org"));
        assert!(fill(&v, page, other).is_none(), "an entry for another origin is refused");
    }
}
