// SPDX-License-Identifier: GPL-3.0-only
//! The vault-side decision for a browser-supplied login. Transport and the
//! person's consent screen must keep this proposal private and call `commit`
//! only after a fresh, explicit choice in Chastity.
use zeroize::Zeroizing;

use crate::vault::{EntryId, NewEntry, Vault};

pub const MAX_FIELD: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal { Scope, Username, Password, Choice }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice { Add, Replace(EntryId) }

/// The only account scope accepted for writes is a canonical HTTPS origin.
/// The browser must derive it from the displayed top-level document. Paths,
/// userinfo, fragments, controls and alternate spellings are refused.
pub fn exact_https_origin(scope: &str) -> bool {
    let Some(host_port) = scope.strip_prefix("https://") else { return false };
    if host_port.is_empty() || host_port.len() > 504
        || host_port.bytes().any(|b| !b.is_ascii_graphic() || matches!(b, b'/' | b'?' | b'#' | b'@' | b'%' | b'\\'))
        || host_port.bytes().any(|b| b.is_ascii_uppercase())
    { return false }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_port, None),
    };
    if host.is_empty() || host.starts_with('.') || host.ends_with('.')
        || host.split('.').any(|part| part.is_empty() || part.starts_with('-') || part.ends_with('-')
            || !part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    { return false }
    match port {
        None => true,
        Some(p) => p.parse::<u16>().is_ok_and(|n| n != 0 && n != 443 && n.to_string() == p),
    }
}

/// No Debug/Clone: a proposal owns a bounded, zeroizing password.
pub struct Proposal {
    origin: String,
    username: String,
    password: Zeroizing<String>,
}
impl Proposal {
    pub fn new(origin: &str, username: &str, password: Zeroizing<String>) -> Result<Self, Refusal> {
        if !exact_https_origin(origin) { return Err(Refusal::Scope); }
        if username.is_empty() || username.len() > MAX_FIELD || username.bytes().any(|b| b == 0) {
            return Err(Refusal::Username);
        }
        if password.is_empty() || password.len() > MAX_FIELD || password.bytes().any(|b| b == 0) {
            return Err(Refusal::Password);
        }
        Ok(Self { origin: origin.into(), username: username.into(), password })
    }

    pub fn origin(&self) -> &str { &self.origin }
    pub fn username(&self) -> &str { &self.username }

    /// Chastity may show only same-origin, same-username targets for update.
    /// The browser cannot choose an entry ID or infer other vault entries.
    pub fn update_targets(&self, vault: &Vault) -> Vec<EntryId> {
        vault.entries().into_iter().filter(|entry| {
            entry.username == self.username && exact_https_origin(&entry.url)
                && entry.url == self.origin
        }).map(|entry| entry.id).collect()
    }

    /// Call only after the person chooses Add or one listed update target.
    /// A stale or cross-origin choice cannot mutate the vault.
    pub fn commit(self, vault: &mut Vault, choice: Choice) -> Result<EntryId, Refusal> {
        match choice {
            Choice::Add => Ok(vault.add(NewEntry {
                title: self.origin.clone(), username: self.username.clone(),
                password: self.password, url: self.origin, ..Default::default()
            })),
            Choice::Replace(id) => {
                if !self.update_targets(vault).contains(&id) { return Err(Refusal::Choice); }
                if vault.replace_password(id, &self.password) { Ok(id) } else { Err(Refusal::Choice) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Kdf;

    fn vault() -> Vault {
        Vault::create("v", &mut "master".into(), Kdf::TEST_ONLY)
    }
    fn proposal(origin: &str, user: &str, pass: &str) -> Result<Proposal, Refusal> {
        Proposal::new(origin, user, Zeroizing::new(pass.into()))
    }

    #[test]
    fn rejects_noncanonical_or_insecure_scopes_and_bad_fields() {
        for scope in ["http://example.org", "https://EXAMPLE.org", "https://example.org/",
            "https://a@example.org", "https://example.org:443", "https://example.org:08080",
            "https://example.org#other", "https://example.org.evil/", "https://-bad.org"] {
            assert!(matches!(proposal(scope, "u", "p"), Err(Refusal::Scope)), "{scope}");
        }
        assert!(exact_https_origin("https://example.org:8080"));
        assert!(matches!(proposal("https://example.org", "", "p"), Err(Refusal::Username)));
        assert!(matches!(proposal("https://example.org", "u", ""), Err(Refusal::Password)));
        assert!(matches!(proposal("https://example.org", "u", &"p".repeat(4097)), Err(Refusal::Password)));
    }

    #[test]
    fn only_selected_exact_account_can_be_updated() {
        let mut v = vault();
        let a = proposal("https://mail.example.org", "alice", "first").unwrap()
            .commit(&mut v, Choice::Add).unwrap();
        let b = proposal("https://mail.example.org", "bob", "other").unwrap()
            .commit(&mut v, Choice::Add).unwrap();
        let c = proposal("https://example.org", "alice", "separate").unwrap()
            .commit(&mut v, Choice::Add).unwrap();
        let request = proposal("https://mail.example.org", "alice", "second").unwrap();
        assert_eq!(request.update_targets(&v), vec![a]);
        assert!(matches!(request.commit(&mut v, Choice::Replace(b)), Err(Refusal::Choice)));
        assert_eq!(v.detail(a).unwrap().password.as_str(), "first");
        assert_eq!(v.detail(b).unwrap().password.as_str(), "other");
        assert_eq!(v.detail(c).unwrap().password.as_str(), "separate");
        proposal("https://mail.example.org", "alice", "second").unwrap()
            .commit(&mut v, Choice::Replace(a)).unwrap();
        assert_eq!(v.detail(a).unwrap().password.as_str(), "second");
        assert_eq!(v.detail(b).unwrap().password.as_str(), "other");
        assert_eq!(v.detail(c).unwrap().password.as_str(), "separate");
    }
}
