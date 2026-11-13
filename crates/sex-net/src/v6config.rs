// SPDX-License-Identifier: GPL-3.0-only
//! How an IPv6-only network tells a machine its resolvers: the RDNSS option of router
//! advertisements (RFC 8106) and stateless DHCPv6 (RFC 8415 Information-Request with
//! the DNS Recursive Name Server option, RFC 3646). Plain data in and out: no sockets,
//! no authority. The engine feeds frames and datagrams through these.

/// Resolvers kept at once, as RFC 8106 suggests (three).
pub const MAX_RESOLVERS: usize = 3;
/// DHCPv6 ports and the all-relay-agents-and-servers group.
pub const CLIENT_PORT: u16 = 546;
pub const SERVER_PORT: u16 = 547;
pub const ALL_SERVERS: [u8; 16] = [0xff, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2];
/// RFC 8415 section 7.6: Information-Request first wait and longest wait, in ms.
pub const INF_TIMEOUT_MS: u64 = 1000;
pub const INF_MAX_RT_MS: u64 = 3600 * 1000;
/// Default information refresh time (RFC 8415 section 21.23), and its floor.
pub const IRT_DEFAULT_S: u32 = 86400;
pub const IRT_MINIMUM_S: u32 = 600;

const INFORMATION_REQUEST: u8 = 11;
const REPLY: u8 = 7;
const OPT_CLIENTID: u16 = 1;
const OPT_SERVERID: u16 = 2;
const OPT_ORO: u16 = 6;
const OPT_ELAPSED: u16 = 8;
const OPT_STATUS: u16 = 13;
const OPT_DNS_SERVERS: u16 = 23;
const OPT_DOMAIN_LIST: u16 = 24;
const OPT_INFO_REFRESH: u16 = 32;

/// What a router advertisement says about configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advert {
    /// M: addresses come from DHCPv6. O: other configuration (DNS) does.
    pub managed: bool,
    pub other: bool,
    /// RDNSS entries: address and lifetime in seconds (0 removes, u32::MAX forever).
    pub resolvers: [Option<([u8; 16], u32)>; MAX_RESOLVERS],
}

fn checksum(src: &[u8], dst: &[u8], next: u8, body: &[u8]) -> u16 {
    let mut s: u32 = 0;
    let mut add = |b: &[u8]| { for c in b.chunks(2) { s += u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32; } };
    add(src); add(dst); add(&(body.len() as u32).to_be_bytes()); add(&[0, 0, 0, next]); add(body);
    while s >> 16 != 0 { s = (s & 0xffff) + (s >> 16); }
    !(s as u16)
}

/// Reads a router advertisement out of an Ethernet frame. None for anything else, and
/// for one no router could have sent (RFC 4861 section 6.1.2: hop limit 255, link-local
/// source, code 0, good checksum) or whose options are malformed.
pub fn router_advert(frame: &[u8]) -> Option<Advert> {
    if frame.len() < 14 + 40 + 16 || frame[12..14] != [0x86, 0xdd] || frame[14] >> 4 != 6 { return None; }
    let ip = &frame[14..];
    let length = u16::from_be_bytes([ip[4], ip[5]]) as usize;
    if ip[6] != 58 || ip[7] != 255 || ip.len() < 40 + length || length < 16 { return None; }
    let (src, dst, body) = (&ip[8..24], &ip[24..40], &ip[40..40 + length]);
    if !(src[0] == 0xfe && src[1] & 0xc0 == 0x80) || body[0] != 134 || body[1] != 0 { return None; }
    if checksum(src, dst, 58, body) != 0 { return None; }
    let mut advert = Advert { managed: body[5] & 0x80 != 0, other: body[5] & 0x40 != 0, ..Advert::default() };
    let mut kept = 0;
    let mut at = 16;
    while at < body.len() {
        let kind = body[at];
        let len = *body.get(at + 1)? as usize * 8;
        if len == 0 || at + len > body.len() { return None; }
        let opt = &body[at..at + len];
        if kind == 25 {
            // RFC 8106 section 5.1: length 3 or more and odd; addresses follow the lifetime.
            if len < 24 || (len / 8) % 2 == 0 { return None; }
            let life = u32::from_be_bytes([opt[4], opt[5], opt[6], opt[7]]);
            for a in opt[8..].chunks_exact(16) {
                if kept < MAX_RESOLVERS { advert.resolvers[kept] = Some((a.try_into().ok()?, life)); kept += 1; }
            }
        }
        at += len;
    }
    Some(advert)
}

/// The DUID this machine names itself by: DUID-LL from its MAC (RFC 8415 section 11.4).
pub fn duid(mac: [u8; 6]) -> [u8; 10] { [0, 3, 0, 1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]] }

/// Bytes of an Information-Request.
pub const REQUEST_BYTES: usize = 4 + 4 + 10 + 4 + 2 + 4 + 4;

/// An Information-Request asking for the DNS servers and search list. `elapsed_ms` is
/// how long this exchange has been going (RFC 8415 section 21.9, in hundredths).
pub fn information_request(xid: u32, mac: [u8; 6], elapsed_ms: u64) -> [u8; REQUEST_BYTES] {
    let mut m = [0u8; REQUEST_BYTES];
    m[0] = INFORMATION_REQUEST;
    m[1..4].copy_from_slice(&xid.to_be_bytes()[1..]);
    let mut at = 4;
    let mut opt = |m: &mut [u8; REQUEST_BYTES], code: u16, data: &[u8]| {
        m[at..at + 2].copy_from_slice(&code.to_be_bytes());
        m[at + 2..at + 4].copy_from_slice(&(data.len() as u16).to_be_bytes());
        m[at + 4..at + 4 + data.len()].copy_from_slice(data);
        at += 4 + data.len();
    };
    opt(&mut m, OPT_CLIENTID, &duid(mac));
    let hundredths = u16::try_from(elapsed_ms / 10).unwrap_or(u16::MAX);
    opt(&mut m, OPT_ELAPSED, &hundredths.to_be_bytes());
    let mut oro = [0u8; 4];
    oro[..2].copy_from_slice(&OPT_DNS_SERVERS.to_be_bytes());
    oro[2..].copy_from_slice(&OPT_DOMAIN_LIST.to_be_bytes());
    opt(&mut m, OPT_ORO, &oro);
    m
}

/// What a DHCPv6 server answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Information {
    pub resolvers: [Option<[u8; 16]>; MAX_RESOLVERS],
    /// When to ask again, in seconds (at least `IRT_MINIMUM_S`).
    pub refresh_s: u32,
}

/// Reads the Reply to Information-Request `xid`. None for anything else: another
/// transaction, a message for another client, a failure status or malformed options.
pub fn reply(xid: u32, mac: [u8; 6], message: &[u8]) -> Option<Information> {
    if message.len() < 4 || message[0] != REPLY || message[1..4] != xid.to_be_bytes()[1..] { return None; }
    let mut info = Information { resolvers: [None; MAX_RESOLVERS], refresh_s: IRT_DEFAULT_S };
    let (mut server, mut kept, mut at) = (false, 0, 4);
    while at < message.len() {
        if at + 4 > message.len() { return None; }
        let code = u16::from_be_bytes([message[at], message[at + 1]]);
        let len = u16::from_be_bytes([message[at + 2], message[at + 3]]) as usize;
        let data = message.get(at + 4..at + 4 + len)?;
        match code {
            OPT_SERVERID => server = !data.is_empty(),
            // A reply naming another client is not ours (section 16.10).
            OPT_CLIENTID if data != duid(mac) => return None,
            OPT_STATUS if data.len() < 2 || data[..2] != [0, 0] => return None,
            OPT_DNS_SERVERS => {
                if len % 16 != 0 { return None; }
                for a in data.chunks_exact(16) {
                    if kept < MAX_RESOLVERS { info.resolvers[kept] = Some(a.try_into().ok()?); kept += 1; }
                }
            }
            OPT_INFO_REFRESH if len == 4 => info.refresh_s = u32::from_be_bytes([data[0], data[1], data[2], data[3]]).max(IRT_MINIMUM_S),
            _ => {}
        }
        at += 4 + len;
    }
    server.then_some(info)
}

/// The wait before the next Information-Request after `previous` ms (RFC 8415 section
/// 15: doubling, capped), with `jitter` in -100..=100 thousandths.
pub fn next_timeout(previous: u64, jitter: i64) -> u64 {
    let base = if previous == 0 { INF_TIMEOUT_MS } else { (previous * 2).min(INF_MAX_RT_MS) };
    let j = jitter.clamp(-100, 100);
    (base as i64 + base as i64 * j / 1000).max(1) as u64
}
