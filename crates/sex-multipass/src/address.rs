// SPDX-License-Identifier: GPL-3.0-only
//! Network account addresses, ported from leeloo-multipass `networkAccount()`.
use crate::net::public_url;
use crate::{ApiError, Result};
use alloc::format;
use alloc::string::{String, ToString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provider {
    Bluesky, Mastodon, Matrix, Fediverse,
    /// CalDAV/CardDAV/WebDAV server (generic, Nextcloud, Fastmail, iCloud app password).
    Dav,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Provider::Bluesky, Provider::Mastodon, Provider::Matrix, Provider::Fediverse, Provider::Dav];
    pub fn as_str(self) -> &'static str {
        match self { Self::Bluesky => "bluesky", Self::Mastodon => "mastodon", Self::Matrix => "matrix", Self::Fediverse => "fediverse", Self::Dav => "dav" }
    }
    pub fn parse(s: &str) -> Option<Self> { Self::ALL.into_iter().find(|p| p.as_str() == s) }
}

/// An address as the stamp book keeps it. There is deliberately no verified flag here:
/// only a completed proof makes a stamp solid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAccount {
    pub provider: Provider,
    pub address: String,
    /// `atproto` for Bluesky, otherwise the server origin.
    pub network: String,
    pub profile_url: String,
}

fn label_ok(l: &str) -> bool {
    !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !l.starts_with('-') && !l.ends_with('-')
}

fn bluesky_handle_ok(h: &str) -> bool {
    if h.len() > 253 { return false; }
    let mut parts = h.split('.').peekable();
    let mut count = 0;
    while let Some(l) = parts.next() {
        count += 1;
        if !label_ok(l) { return false; }
        if parts.peek().is_none() {
            // The TLD starts with a letter and is at least two characters.
            if l.len() < 2 || !l.as_bytes()[0].is_ascii_lowercase() { return false; }
        }
    }
    count >= 2
}

fn server_ok(s: &str) -> bool { !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') }

/// `@?name@server` → (name, server).
fn split_fed(addr: &str) -> Option<(&str, &str)> {
    let a = addr.strip_prefix('@').unwrap_or(addr);
    let (n, s) = a.split_once('@')?;
    if n.is_empty() || n.contains(char::is_whitespace) || !server_ok(s) { return None; }
    Some((n, s))
}

pub fn network_account(provider: Provider, raw: &str) -> Result<NetworkAccount> {
    let address = raw.trim();
    match provider {
        Provider::Bluesky => {
            let handle = address.strip_prefix('@').unwrap_or(address).to_ascii_lowercase();
            if !bluesky_handle_ok(&handle) {
                return Err(ApiError::new("invalid_handle", "Use a full Bluesky handle, such as alice.example.social."));
            }
            Ok(NetworkAccount { provider, profile_url: format!("https://bsky.app/profile/{handle}"), address: handle, network: "atproto".into() })
        }
        Provider::Matrix => {
            let bad = || ApiError::new("invalid_mxid", "Use a full Matrix address, such as @alice:matrix.example.");
            let rest = address.strip_prefix('@').ok_or_else(bad)?;
            let (local, server) = rest.split_once(':').ok_or_else(bad)?;
            if local.is_empty() || local.contains(char::is_whitespace) || !server_ok(server) { return Err(bad()); }
            let url = public_url(&format!("https://{}", server.to_ascii_lowercase()))?;
            Ok(NetworkAccount {
                provider,
                address: format!("@{local}:{}", url.host),
                network: url.origin(),
                profile_url: format!("https://matrix.to/#/{}", encode_component(address)),
            })
        }
        Provider::Mastodon => {
            let bad = || ApiError::new("invalid_mastodon", "Use @name@server or your HTTPS profile URL.");
            let (username, server) = if address.starts_with("https://") {
                let url = public_url(address)?;
                let p = url.path.split('?').next().unwrap_or("");
                let name = p.strip_prefix("/@").unwrap_or(p.trim_start_matches('/')).trim_end_matches('/').to_string();
                (name, url.host)
            } else {
                let (n, s) = split_fed(address).ok_or_else(bad)?;
                if !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err(bad()); }
                (n.to_string(), s.to_string())
            };
            if username.is_empty() || !username.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(ApiError::new("invalid_mastodon", "Use the account’s @name profile URL."));
            }
            let url = public_url(&format!("https://{}", server.to_ascii_lowercase()))?;
            let network = url.origin();
            Ok(NetworkAccount { provider, address: format!("@{username}@{}", url.host), profile_url: format!("{network}/@{username}"), network })
        }
        Provider::Dav => {
            // `user@https://server/path` is how a DAV account is written down.
            let bad = || ApiError::new("invalid_dav", "Use your user name and the server's HTTPS address.");
            let (user, url) = address.split_once("@https://").ok_or_else(bad)?;
            server_account(Provider::Dav, &format!("https://{url}"), user)
        }
        Provider::Fediverse => {
            // A URL is a profile URL even when its path holds an `@` (leeloo's pattern
            // would read `https://host/@x` as the account "https://host/" on server "x").
            if let Some((n, s)) = split_fed(address).filter(|_| !address.contains("://")) {
                let url = public_url(&format!("https://{}", s.to_ascii_lowercase()))?;
                let network = url.origin();
                return Ok(NetworkAccount { provider, address: format!("@{n}@{}", url.host), profile_url: format!("{network}/@{}", encode_component(n)), network });
            }
            let mut url = public_url(address)?;
            url.fragment = None;
            Ok(NetworkAccount { provider, address: url.href(), network: url.origin(), profile_url: url.href() })
        }
    }
}

/// An account on a server reached with a user name and an app password (DAV now; mail
/// servers in Y2). The address is `user@host`; the profile URL is the service base URL.
pub fn server_account(provider: Provider, base_url: &str, user: &str) -> Result<NetworkAccount> {
    if user.is_empty() || user.len() > 320 || user.bytes().any(|b| b <= b' ' || b == b':' || b == 0x7f) {
        return Err(ApiError::new("invalid_user", "Use the account's user name."));
    }
    let url = public_url(base_url)?;
    let mut base = url.href();
    if url.fragment.is_some() || base.contains('?') { return Err(ApiError::new("invalid_network", "Use the server's base address without a query.")); }
    if !base.ends_with('/') { base.push('/'); }
    Ok(NetworkAccount { provider, address: format!("{user}@{base}"), network: url.origin(), profile_url: base })
}

/// `encodeURIComponent`.
pub fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) { out.push(b as char); } else { out.push_str(&format!("%{b:02X}")); }
    }
    out
}

/// `application/x-www-form-urlencoded` value encoding.
pub fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.*".contains(&b) { out.push(b as char); }
        else if b == b' ' { out.push('+'); }
        else { out.push_str(&format!("%{b:02X}")); }
    }
    out
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 { out.push('&'); }
        out.push_str(&form_encode(k)); out.push('='); out.push_str(&form_encode(v));
    }
    out
}

/// Read one query parameter from a URL or query string.
pub fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?').map(|x| x.1).unwrap_or(url);
    let q = q.split('#').next().unwrap_or(q);
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| percent_decode(v))
    })
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = alloc::vec::Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(core::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => { out.push(v); i += 2; }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from leeloo-multipass "account parsing keeps stable network namespaces and no implicit verified flag".
    #[test]
    fn stable_namespaces() {
        assert_eq!(network_account(Provider::Bluesky, "@alice.example.social").unwrap().address, "alice.example.social");
        assert_eq!(network_account(Provider::Matrix, "@alice:matrix.example").unwrap().network, "https://matrix.example");
        assert_eq!(network_account(Provider::Mastodon, "https://social.example/@alice").unwrap().address, "@alice@social.example");
        assert_eq!(network_account(Provider::Fediverse, "@alice@threads.net").unwrap().network, "https://threads.net");
        let m = network_account(Provider::Mastodon, "@Bob_1@Social.Example").unwrap();
        assert_eq!((m.address.as_str(), m.profile_url.as_str()), ("@Bob_1@social.example", "https://social.example/@Bob_1"));
        assert_eq!(network_account(Provider::Matrix, "@a:m.example").unwrap().profile_url, "https://matrix.to/#/%40a%3Am.example");
    }

    #[test]
    fn bad_addresses_have_their_codes() {
        assert_eq!(network_account(Provider::Bluesky, "alice").unwrap_err().code, "invalid_handle");
        assert_eq!(network_account(Provider::Bluesky, "alice.1x").unwrap_err().code, "invalid_handle");
        assert_eq!(network_account(Provider::Matrix, "alice:m.example").unwrap_err().code, "invalid_mxid");
        assert_eq!(network_account(Provider::Matrix, "@alice:localhost").unwrap_err().code, "invalid_network");
        assert_eq!(network_account(Provider::Mastodon, "@al-ice@social.example").unwrap_err().code, "invalid_mastodon");
        assert_eq!(network_account(Provider::Fediverse, "https://10.0.0.1/@x").unwrap_err().code, "invalid_network");
    }

    #[test]
    fn dav_accounts() {
        let a = network_account(Provider::Dav, "alice@https://cloud.example/remote.php/dav").unwrap();
        assert_eq!((a.address.as_str(), a.network.as_str(), a.profile_url.as_str()),
            ("alice@https://cloud.example/remote.php/dav/", "https://cloud.example", "https://cloud.example/remote.php/dav/"));
        assert_eq!(network_account(Provider::Dav, "alice@http://cloud.example").unwrap_err().code, "invalid_dav");
        assert_eq!(server_account(Provider::Dav, "https://c.example", "a:b").unwrap_err().code, "invalid_user");
    }

    #[test]
    fn query_and_form() {
        assert_eq!(query_param("https://x/cb?state=a%20b&code=c+d", "state").as_deref(), Some("a b"));
        assert_eq!(query_param("https://x/cb?state=a&code=c+d", "code").as_deref(), Some("c d"));
        assert_eq!(form(&[("a b", "c&d")]), "a+b=c%26d");
    }
}
