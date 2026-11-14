// SPDX-License-Identifier: GPL-3.0-only
//! Anonymous mode: one policy object the rest of the engine asks before it says
//! anything about itself.
//!
//! When [`AnonymousPolicy::enabled`] is set, the engine must:
//! - send no client name or version: no `v` key in the extension handshake (BEP 10), no
//!   `User-Agent` header to HTTP trackers or web seeds ([`AnonymousPolicy::user_agent`]
//!   is `None`), and a peer ID whose prefix is random instead of `-SX0100-`;
//! - not use DHT, Local Service Discovery or Peer Exchange (each would publish our
//!   address and the torrents we have to strangers or to the LAN);
//! - not send `ip`, `ipv4`/`ipv6` or a `localip` in tracker announces, nor our own
//!   address or listen port (`p`) in the extension handshake;
//! - reach trackers and web seeds only through the configured proxy or tunnel, never
//!   directly ([`AnonymousPolicy::allow_tracker`]);
//! - optionally refuse incoming connections and skip NAT port mapping, so the machine
//!   does not advertise an open port.
//!
//! What it does not do: it does not hide our IP address from the peers we connect to,
//! or from the trackers behind the proxy; that takes a tunnel (the program's network
//! grant, e.g. WireGuard only). It does not encrypt traffic (see `mse`), does not hide
//! which torrents we transfer from someone watching the tunnel's exit, and cannot stop
//! a .torrent's own contents (comment, creator) from identifying who made it. Timing,
//! piece-request patterns and the set of extensions we support can still fingerprint a
//! client. [`audit`] is a last-line check for our own name in outgoing bytes, not a
//! proof of anonymity.
use crate::{PeerId, CLIENT_PREFIX};

/// The client's name as it appears in normal (non-anonymous) mode.
pub const CLIENT_NAME: &str = "Swingers";
/// The client's version.
pub const CLIENT_VERSION: &str = "0.1.0";
/// The extension-handshake `v` value in normal mode.
pub const CLIENT_STRING: &str = "Swingers 0.1.0";
/// The HTTP `User-Agent` in normal mode.
pub const USER_AGENT: &str = "Swingers/0.1.0";

/// The anonymous-mode policy. [`Default`] is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AnonymousPolicy {
    /// Anonymous mode on.
    pub enabled: bool,
    /// Refuse incoming peer connections (and do no port mapping).
    pub refuse_incoming: bool,
    /// Whether a proxy or tunnel is configured for tracker and web seed traffic.
    pub proxy_configured: bool,
}

impl AnonymousPolicy {
    /// Normal mode.
    pub fn off() -> AnonymousPolicy { AnonymousPolicy::default() }
    /// Anonymous mode, refusing incoming connections, with a proxy configured or not.
    pub fn on(proxy_configured: bool) -> AnonymousPolicy {
        AnonymousPolicy { enabled: true, refuse_incoming: true, proxy_configured }
    }
    /// The extension handshake's `v` value; `None` means omit the key.
    pub fn client_version(&self) -> Option<&'static str> { (!self.enabled).then_some(CLIENT_STRING) }
    /// The HTTP `User-Agent`; `None` means send no such header.
    pub fn user_agent(&self) -> Option<&'static str> { (!self.enabled).then_some(USER_AGENT) }
    /// A fresh peer ID: random prefix in anonymous mode.
    pub fn peer_id(&self, random: &mut dyn FnMut(&mut [u8])) -> PeerId {
        let mut id = PeerId::generate(self.enabled, random);
        if self.enabled {
            // Make sure random bytes cannot spell our tag or name by chance: setting the
            // high bit of a match's first byte breaks it and cannot create a new match.
            let name = CLIENT_NAME.as_bytes();
            for i in 0..20 {
                let w = &id.0[i..];
                let tag = w.len() >= 4 && w[..3] == CLIENT_PREFIX[..3] && w[3].is_ascii_digit();
                let named = w.len() >= name.len() && w[..name.len()].eq_ignore_ascii_case(name);
                if tag || named { id.0[i] |= 0x80; }
            }
        }
        id
    }
    pub fn dht_allowed(&self) -> bool { !self.enabled }
    pub fn lsd_allowed(&self) -> bool { !self.enabled }
    pub fn pex_allowed(&self) -> bool { !self.enabled }
    /// Whether announces may carry `ip`/`ipv4`/`ipv6`/`localip`.
    pub fn announce_ip_allowed(&self) -> bool { !self.enabled }
    /// Whether the extension handshake may carry our listen port `p` and addresses.
    pub fn handshake_addresses_allowed(&self) -> bool { !self.enabled }
    /// Whether to accept incoming peer connections.
    pub fn accept_incoming(&self) -> bool { !(self.enabled && self.refuse_incoming) }
    /// Whether NAT-PMP/PCP/UPnP port mapping may run.
    pub fn port_mapping_allowed(&self) -> bool { self.accept_incoming() }
    /// Whether a tracker (or web seed) request may go out by this route.
    pub fn allow_tracker(&self, via_proxy: bool) -> bool { !self.enabled || (via_proxy && self.proxy_configured) }
    /// Whether a web seed may be used by this route.
    pub fn allow_web_seed(&self, via_proxy: bool) -> bool { self.allow_tracker(via_proxy) }
    /// Checks an outgoing buffer: `true` when it is clean, or anonymous mode is off.
    pub fn check(&self, bytes: &[u8]) -> bool { !self.enabled || audit(bytes) }
}

/// `true` when `bytes` contain none of our identifying strings: the client name
/// (any case), the peer-ID tag `-SX` followed by a version digit, the `User-Agent` or
/// the handshake version string. Random data matches `-SX<digit>` with odds around 1
/// in 4 million per position, so a hit on a random buffer is possible but rare.
pub fn audit(bytes: &[u8]) -> bool {
    let name = CLIENT_NAME.as_bytes();
    let has_name = bytes.windows(name.len()).any(|w| w.eq_ignore_ascii_case(name));
    let tag = &CLIENT_PREFIX[..3];
    let has_tag = bytes.windows(4).any(|w| &w[..3] == tag && w[3].is_ascii_digit());
    !(has_name || has_tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bencode::{dict, Item};
    use alloc::vec::Vec;

    fn ext_handshake(p: &AnonymousPolicy) -> Vec<u8> {
        // The shape of a BEP 10 handshake as the extension module builds it.
        let mut entries: Vec<(&[u8], Item)> = Vec::new();
        entries.push((b"m", dict([(b"ut_metadata".as_slice(), Item::Int(1))])));
        if p.handshake_addresses_allowed() { entries.push((b"p", Item::Int(51413))); }
        if let Some(v) = p.client_version() { entries.push((b"v", Item::Bytes(v.as_bytes().to_vec()))); }
        let mut d = alloc::collections::BTreeMap::new();
        for (k, v) in entries { d.insert(k.to_vec(), v); }
        Item::Dict(d).encode()
    }

    #[test]
    fn normal_mode_identifies_itself() {
        let p = AnonymousPolicy::off();
        assert_eq!(p.user_agent(), Some("Swingers/0.1.0"));
        assert!(!audit(&ext_handshake(&p)));
        let mut fill = |b: &mut [u8]| b.fill(0x41);
        assert!(!audit(&p.peer_id(&mut fill).0));
        assert!(p.dht_allowed() && p.lsd_allowed() && p.pex_allowed() && p.accept_incoming() && p.allow_tracker(false));
        assert!(p.check(b"Swingers"));
    }

    #[test]
    fn anonymous_mode_gate() {
        let p = AnonymousPolicy::on(true);
        assert_eq!((p.user_agent(), p.client_version()), (None, None));
        let hs = ext_handshake(&p);
        assert!(audit(&hs) && p.check(&hs));
        assert!(!hs.windows(3).any(|w| w == b"1:p"));
        // Peer IDs never carry the tag, even when the random source is adversarial.
        let mut adversarial = |b: &mut [u8]| { for (i, x) in b.iter_mut().enumerate() { *x = b"-SX0100-"[i % 8]; } };
        let id = p.peer_id(&mut adversarial);
        assert!(audit(&id.0), "{:?}", id);
        let mut seed = 7u32;
        let mut rnd = |b: &mut [u8]| for x in b.iter_mut() { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); *x = (seed >> 24) as u8; };
        for _ in 0..1000 { assert!(audit(&p.peer_id(&mut rnd).0)); }
        assert!(!p.dht_allowed() && !p.lsd_allowed() && !p.pex_allowed() && !p.announce_ip_allowed());
        assert!(!p.accept_incoming() && !p.port_mapping_allowed());
        assert!(p.allow_tracker(true) && !p.allow_tracker(false) && !p.allow_web_seed(false));
        assert!(!AnonymousPolicy::on(false).allow_tracker(true));
        let p2 = AnonymousPolicy { refuse_incoming: false, ..p };
        assert!(p2.accept_incoming());
    }

    #[test]
    fn audit_finds_our_strings() {
        assert!(!audit(b"GET /announce HTTP/1.1\r\nUser-Agent: swingers/0.1.0\r\n"));
        assert!(!audit(b"\x13BitTorrent protocol........-SX0100-abcdefghijkl"));
        assert!(!audit(b"xx-SX9yy"));
        assert!(audit(b"-SXa -sx0 SWINGER Swinger"));
        assert!(audit(b""));
        assert!(audit(b"GET /announce?info_hash=%aa&peer_id=%12%34 HTTP/1.1\r\nHost: t\r\n\r\n"));
    }
}
