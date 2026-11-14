// SPDX-License-Identifier: GPL-3.0-only
//! Outbound address rules, ported from leeloo-multipass `network-http.ts`: credentials only
//! go to public HTTPS servers on port 443, never to loopback, private, link-local or other
//! reserved addresses. The network service checks the resolved address of every flow with
//! [`public_address`] too, so a name that resolves to a private address is refused
//! (no DNS rebinding).
use crate::{ApiError, Result};
use alloc::format;
use alloc::string::{String, ToString};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A checked public HTTPS URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrl {
    /// Lower-case host, without brackets.
    pub host: String,
    /// Path and query, starting with `/`.
    pub path: String,
    pub fragment: Option<String>,
}

impl PublicUrl {
    pub fn origin(&self) -> String {
        if self.host.contains(':') { format!("https://[{}]", self.host) } else { format!("https://{}", self.host) }
    }
    pub fn href(&self) -> String {
        let mut s = self.origin();
        s.push_str(&self.path);
        if let Some(f) = &self.fragment { s.push('#'); s.push_str(f); }
        s
    }
}

fn invalid() -> ApiError { ApiError::new("invalid_network", "Network connections require a public HTTPS server.") }

pub fn public_url(value: &str) -> Result<PublicUrl> {
    let rest = value.strip_prefix("https://").ok_or_else(|| {
        if value.contains("://") { invalid() } else { ApiError::new("invalid_network", "Use a public HTTPS server or profile URL.") }
    })?;
    let (rest, fragment) = match rest.split_once('#') { Some((a, f)) => (a, Some(f.to_string())), None => (rest, None) };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let mut path = rest[end..].to_string();
    if path.is_empty() || path.starts_with('?') { path.insert(0, '/'); }
    if authority.is_empty() || authority.contains('@') { return Err(invalid()); }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let close = inner.find(']').ok_or_else(invalid)?;
        let after = &inner[close + 1..];
        (&inner[..close], after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') { Some((h, p)) => (h, Some(p)), None => (authority, None) }
    };
    if let Some(p) = port { if p != "443" && !p.is_empty() { return Err(invalid()); } }
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") { return Err(invalid()); }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !public_ip(ip) { return Err(invalid()); }
    } else if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') || host.starts_with('.') || host.contains("..") {
        return Err(invalid());
    }
    Ok(PublicUrl { host, path, fragment })
}

/// Whether an address (text form) is ordinary public unicast.
pub fn public_address(address: &str) -> bool {
    address.parse::<IpAddr>().map(public_ip).unwrap_or(false)
}

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() { return public_v4(v4); }
            public_v6(v6)
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0                                   // this network
        || a == 10                             // private
        || a == 127                            // loopback
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
        || (a == 169 && b == 254)              // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b)) // private
        || (a == 192 && b == 0 && c == 0)      // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)      // TEST-NET-1
        || (a == 192 && b == 88 && c == 99)    // 6to4 relay
        || (a == 192 && b == 168)              // private
        || (a == 198 && (18..20).contains(&b)) // benchmarking
        || (a == 198 && b == 51 && c == 100)   // TEST-NET-2
        || (a == 203 && b == 0 && c == 113)    // TEST-NET-3
        || a >= 224)                           // multicast, reserved, broadcast
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || (s[0] & 0xfe00) == 0xfc00            // unique local
        || (s[0] & 0xffc0) == 0xfe80            // link-local
        || (s[0] & 0xff00) == 0xff00            // multicast
        || (s[0] == 0x2001 && s[1] == 0x0db8)   // documentation
        || (s[0] == 0x2001 && s[1] == 0)        // Teredo
        || s[0] == 0x2002                       // 6to4
        || (s[0] == 0x64 && s[1] == 0xff9b)     // NAT64
        || (s[0] == 0x100 && s[1] == 0 && s[2] == 0 && s[3] == 0) // discard
        || (s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0)) // IPv4-compatible
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from leeloo-multipass test/core.test.ts "outbound HTTP blocks private destinations".
    #[test]
    fn private_destinations_and_unsafe_forms_are_refused() {
        for a in ["127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "fc00::1", "::ffff:127.0.0.1", "100.64.0.1", "fe80::1"] {
            assert!(!public_address(a), "{a}");
        }
        assert!(public_address("8.8.8.8"));
        assert!(public_address("2606:4700::1111"));
        for u in ["http://public.example", "https://localhost", "https://127.0.0.1", "https://name:secret@public.example", "https://public.example:8443", "https://a.localhost/x", "https://[::1]/"] {
            assert!(public_url(u).is_err(), "{u}");
        }
    }

    #[test]
    fn public_urls_normalise() {
        let u = public_url("https://Social.Example:443/@alice?x=1#frag").unwrap();
        assert_eq!(u.host, "social.example");
        assert_eq!(u.origin(), "https://social.example");
        assert_eq!(u.path, "/@alice?x=1");
        assert_eq!(public_url("https://matrix.example").unwrap().path, "/");
    }
}
